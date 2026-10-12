use crate::sensor::{PINS, Reading, Sensor};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Memory::{
    FILE_MAP_READ, MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
    UnmapViewOfFile, VirtualQuery,
};
use windows_sys::Win32::System::Threading::{
    MUTEX_MODIFY_STATE, OpenMutexW, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject,
};

const MAPPING: &str = r"Global\HWiNFO_SENS_SM2";
const MUTEX: &str = r"Global\HWiNFO_SM2_MUTEX";
const MAGIC: u32 = u32::from_le_bytes(*b"HWiS");
const STALE_SECS: i64 = 30;
const MUTEX_WAIT_MS: u32 = 200;
const HEADER_POLL_TIME: usize = 12;
const HEADER_ENTRY_OFFSET: usize = 32;
const HEADER_ENTRY_SIZE: usize = 36;
const HEADER_ENTRY_COUNT: usize = 40;
const LABEL_LEN: usize = 128;
const LABEL_ORIG: usize = 12;
const LABEL_USER: usize = LABEL_ORIG + LABEL_LEN;
const UNIT: usize = LABEL_USER + LABEL_LEN;
const UNIT_LEN: usize = 16;
const VALUE: usize = UNIT + UNIT_LEN;
const ENTRY_MIN: usize = VALUE + 8;

struct View {
    map: HANDLE,
    base: *const u8,
    len: usize,
    mutex: HANDLE,
}

struct Locked<'a>(&'a View);

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        if !self.0.mutex.is_null() {
            unsafe { ReleaseMutex(self.0.mutex) };
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

impl View {
    fn open() -> Result<Self, String> {
        let map = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, wide(MAPPING).as_ptr()) };
        if map.is_null() {
            return Err("not running or Shared Memory Support is off".into());
        }

        let base = unsafe { MapViewOfFile(map, FILE_MAP_READ, 0, 0, 0) }.Value as *const u8;
        if base.is_null() {
            unsafe { CloseHandle(map) };
            return Err(format!("MapViewOfFile: {}", std::io::Error::last_os_error()));
        }

        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        unsafe { VirtualQuery(base.cast(), &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) };

        let mutex = unsafe {
            OpenMutexW(
                SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
                0,
                wide(MUTEX).as_ptr(),
            )
        };

        Ok(Self {
            map,
            base,
            len: info.RegionSize,
            mutex,
        })
    }

    fn lock(&self) -> Result<Locked<'_>, String> {
        if self.mutex.is_null() {
            return Ok(Locked(self));
        }

        match unsafe { WaitForSingleObject(self.mutex, MUTEX_WAIT_MS) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Locked(self)),
            _ => Err("shared memory busy".into()),
        }
    }

    fn bytes(&self, at: usize, len: usize) -> Result<&[u8], String> {
        match at.checked_add(len) {
            Some(end) if end <= self.len => Ok(unsafe { std::slice::from_raw_parts(self.base.add(at), len) }),
            _ => Err("shared memory layout out of bounds".into()),
        }
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        self.bytes(at, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64(&self, at: usize) -> Result<i64, String> {
        self.bytes(at, 8)
            .map(|b| i64::from_le_bytes(b.try_into().unwrap_or_default()))
    }

    fn f64(&self, at: usize) -> Result<f64, String> {
        self.bytes(at, 8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap_or_default()))
    }
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.base.cast_mut().cast(),
            });
            CloseHandle(self.map);
            if !self.mutex.is_null() {
                CloseHandle(self.mutex);
            }
        }
    }
}

fn cstr(field: &[u8]) -> &[u8] {
    field.split(|&b| b == 0).next().unwrap_or_default()
}

#[derive(Clone, Copy)]
struct Slots {
    amps: [usize; PINS],
    volts: [Option<usize>; PINS],
}

pub struct HwInfo {
    view: Option<View>,
    slots: Option<Slots>,
    amp_labels: [String; PINS],
    volt_labels: [String; PINS],
}

impl HwInfo {
    pub fn new() -> Self {
        Self {
            view: None,
            slots: None,
            amp_labels: std::array::from_fn(|i| format!("GPU 12VHPWR Pin{} Current", i + 1)),
            volt_labels: std::array::from_fn(|i| format!("GPU 12VHPWR Pin{} Voltage", i + 1)),
        }
    }

    fn labelled(view: &View, entry: usize, label: &str) -> bool {
        let matches = |at: usize| {
            view.bytes(entry + at, LABEL_LEN)
                .is_ok_and(|b| cstr(b) == label.as_bytes())
        };
        matches(LABEL_ORIG) || matches(LABEL_USER)
    }

    fn locate(view: &View, amp_labels: &[String; PINS], volt_labels: &[String; PINS]) -> Result<Slots, String> {
        let offset = view.u32(HEADER_ENTRY_OFFSET)? as usize;
        let size = view.u32(HEADER_ENTRY_SIZE)? as usize;
        let count = view.u32(HEADER_ENTRY_COUNT)? as usize;

        if size < ENTRY_MIN {
            return Err(format!("unsupported entry size {size}"));
        }

        let entries: Vec<usize> = (0..count).map(|i| offset + i * size).collect();
        let find = |label: &str| entries.iter().copied().find(|&e| Self::labelled(view, e, label));

        let mut amps = [0; PINS];
        for (slot, label) in amps.iter_mut().zip(amp_labels) {
            *slot = find(label).ok_or_else(|| format!("no \"{label}\" sensor"))?;
        }

        Ok(Slots {
            amps,
            volts: std::array::from_fn(|i| find(&volt_labels[i])),
        })
    }

    fn value(view: &View, entry: usize, milli: &[u8]) -> Result<f32, String> {
        let unit = cstr(view.bytes(entry + UNIT, UNIT_LEN)?);
        let value = view.f64(entry + VALUE)? as f32;
        Ok(if unit == milli { value / 1000.0 } else { value })
    }

    fn sample(&mut self) -> Result<Reading, String> {
        let view = match self.view.take() {
            Some(view) => view,
            None => View::open()?,
        };
        let view = self.view.insert(view);
        let _locked = view.lock()?;

        if view.u32(0)? != MAGIC {
            return Err("HWiNFO has exited".into());
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if now - view.i64(HEADER_POLL_TIME)? > STALE_SECS {
            return Err("HWiNFO data is stale".into());
        }

        let fresh = self.slots.filter(|s| {
            s.amps
                .iter()
                .zip(&self.amp_labels)
                .all(|(&e, label)| Self::labelled(view, e, label))
        });
        let slots = match fresh {
            Some(slots) => slots,
            None => Self::locate(view, &self.amp_labels, &self.volt_labels)?,
        };
        self.slots = Some(slots);

        let mut reading = Reading {
            volts: [12.0; PINS],
            amps: [0.0; PINS],
        };

        for pin in 0..PINS {
            reading.amps[pin] = Self::value(view, slots.amps[pin], b"mA")?;
            if let Some(entry) = slots.volts[pin] {
                reading.volts[pin] = Self::value(view, entry, b"mV")?;
            }
        }

        match reading.plausible() {
            true => Ok(reading),
            false => Err(format!("implausible values {reading}")),
        }
    }
}

impl Sensor for HwInfo {
    fn read(&mut self) -> Result<Reading, String> {
        self.sample().inspect_err(|_| {
            self.view = None;
            self.slots = None;
        })
    }
}
