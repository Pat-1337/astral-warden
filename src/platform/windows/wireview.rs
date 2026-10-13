use super::{os_error, registry, wide};
use crate::sensor::Sensor;
use crate::wireview::{self, Port, USB_PRODUCT, USB_VENDOR, WireView};
use windows_sys::Win32::Devices::Communication::{
    COMMTIMEOUTS, DCB, GetCommState, NOPARITY, ONESTOPBIT, PURGE_RXCLEAR, PurgeComm, SetCommState, SetCommTimeouts,
};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING, ReadFile, WriteFile};

const BAUD: u32 = 115_200;
const TIMEOUT_MS: u32 = 1000;
const BINARY_DTR_RTS: u32 = 0x1011;

pub type Opened = (Box<dyn Sensor>, &'static str, String);

struct Com(HANDLE);

impl Com {
    fn open(port: &str) -> Result<Self, String> {
        let path = wide(&format!(r"\\.\{port}"));
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("{port}: {} (close other WireView software)", os_error()));
        }

        let com = Self(handle);
        let mut dcb = unsafe { std::mem::zeroed::<DCB>() };
        dcb.DCBlength = size_of::<DCB>() as u32;

        let timeouts = COMMTIMEOUTS {
            ReadIntervalTimeout: 0,
            ReadTotalTimeoutMultiplier: 0,
            ReadTotalTimeoutConstant: TIMEOUT_MS,
            WriteTotalTimeoutMultiplier: 0,
            WriteTotalTimeoutConstant: TIMEOUT_MS,
        };

        let ok = unsafe { GetCommState(handle, &mut dcb) } != 0 && {
            dcb.BaudRate = BAUD;
            dcb._bitfield = BINARY_DTR_RTS;
            dcb.ByteSize = 8;
            dcb.Parity = NOPARITY;
            dcb.StopBits = ONESTOPBIT;
            unsafe { SetCommState(handle, &dcb) != 0 && SetCommTimeouts(handle, &timeouts) != 0 }
        };

        match ok {
            true => Ok(com),
            false => Err(format!("{port}: {}", os_error())),
        }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Port for Com {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        let mut written = 0;
        let ok = unsafe {
            WriteFile(
                self.0,
                data.as_ptr(),
                data.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };

        match ok != 0 && written as usize == data.len() {
            true => Ok(()),
            false => Err(format!("WireView write: {}", os_error())),
        }
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), String> {
        let mut filled = 0;

        while filled < buf.len() {
            let mut read = 0;
            let rest = &mut buf[filled..];
            if unsafe {
                ReadFile(
                    self.0,
                    rest.as_mut_ptr(),
                    rest.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(format!("WireView read: {}", os_error()));
            }

            if read == 0 {
                return Err("WireView did not answer".into());
            }
            filled += read as usize;
        }

        Ok(())
    }

    fn discard(&mut self) {
        unsafe { PurgeComm(self.0, PURGE_RXCLEAR) };
    }
}

fn ports() -> Vec<String> {
    registry::usb_ports(&format!("VID_{USB_VENDOR:04X}&PID_{USB_PRODUCT:04X}"))
}

fn probe(port: &str) -> Result<(Com, &'static str), String> {
    Com::open(port).and_then(wireview::probe)
}

fn sensor(com: Com, model: &'static str, port: &str, reopen: impl FnMut() -> Result<Com, String> + 'static) -> Opened {
    (Box::new(WireView::new(com, reopen)), model, format!("via {port}"))
}

fn reopen() -> Result<Com, String> {
    ports()
        .iter()
        .find_map(|port| probe(port).ok())
        .map(|(com, _)| com)
        .ok_or_else(|| "WireView Pro II unplugged".into())
}

pub fn open(port: Option<&str>) -> Option<Result<Opened, String>> {
    if let Some(port) = port {
        let owned = port.to_owned();
        let reopen = move || probe(&owned).map(|(com, _)| com);
        return Some(probe(port).map(|(com, model)| sensor(com, model, port, reopen)));
    }

    let mut failure = None;
    for port in ports() {
        match probe(&port) {
            Ok((com, model)) => return Some(Ok(sensor(com, model, &port, reopen))),
            Err(e) => failure = Some(Err(e)),
        }
    }

    failure
}
