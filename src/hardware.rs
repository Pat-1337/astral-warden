use crate::sensor::{PINS, Reading, Sensor};

pub const NVIDIA: u16 = 0x10DE;
pub const ASUS: u16 = 0x1043;

pub const IT8915_ADDR: u8 = 0x2B;
pub const IT8915_CHIP_REG: u8 = 0x21;
pub const IT8915_CHIP_ID: u8 = 0x89;
pub const IT8915_PINS_REG: u8 = 0x80;
pub const FRAME_LEN: usize = PINS * 4;
const ATTEMPTS: usize = 3;
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

const GPUS: &[(u16, &str)] = &[
    (0x2B85, "RTX 5090"),
    (0x2B87, "RTX 5090 D"),
    (0x2B8C, "RTX 5090 D v2"),
    (0x2C02, "RTX 5080"),
];

const BOARDS: &[(u16, &str)] = &[
    (0x89E3, "ROG Astral OC"),
    (0x89E4, "ROG Astral"),
    (0x89EA, "ROG Astral D OC"),
    (0x89EC, "ROG Astral LC"),
    (0x89ED, "ROG Astral LC OC"),
    (0x8A2E, "ROG Astral OC White"),
    (0x8A3C, "ROG Astral BTF OC"),
    (0x8A5A, "ROG Astral BTF OC"),
    (0x8A61, "ROG Matrix Platinum"),
    (0x89DE, "ROG Astral OC"),
    (0x89DF, "ROG Astral"),
    (0x8A2B, "ROG Astral OC White"),
    (0x8A2C, "ROG Astral White"),
    (0x8A45, "ROG Astral OC Hatsune Miku"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciId {
    pub vendor: u16,
    pub device: u16,
    pub sub_vendor: u16,
    pub sub_device: u16,
}

impl PciId {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn from_dwords(device_id: u32, subsystem_id: u32) -> Self {
        Self {
            vendor: device_id as u16,
            device: (device_id >> 16) as u16,
            sub_vendor: subsystem_id as u16,
            sub_device: (subsystem_id >> 16) as u16,
        }
    }

    pub fn dwords(self) -> (u32, u32) {
        (
            u32::from(self.device) << 16 | u32::from(self.vendor),
            u32::from(self.sub_device) << 16 | u32::from(self.sub_vendor),
        )
    }

    pub fn model(self) -> Option<String> {
        if (self.vendor, self.sub_vendor) != (NVIDIA, ASUS) {
            return None;
        }

        let gpu = GPUS.iter().find(|g| g.0 == self.device)?.1;
        let board = BOARDS.iter().find(|b| b.0 == self.sub_device)?.1;

        Some(format!("ASUS {board} {gpu} [{self}]"))
    }
}

impl std::fmt::Display for PciId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04x}:{:04x} {:04x}:{:04x}",
            self.vendor, self.device, self.sub_vendor, self.sub_device
        )
    }
}

pub fn no_device(failures: &[String]) -> String {
    match failures {
        [] => "no supported ASUS ROG Astral GPU found".into(),
        _ => format!("no usable ASUS ROG Astral GPU: {}", failures.join("; ")),
    }
}

pub fn decode(frame: &[u8; FRAME_LEN]) -> Result<Reading, String> {
    let be = |i: usize| f32::from(u16::from_be_bytes([frame[i], frame[i + 1]])) / 1000.0;
    let mut reading = Reading::default();

    for pin in 0..PINS {
        let base = (PINS - 1 - pin) * 4;
        reading.volts[pin] = be(base);
        reading.amps[pin] = be(base + 2);
    }

    if reading.plausible() {
        Ok(reading)
    } else {
        Err(format!("implausible frame {reading}"))
    }
}

pub trait Bus {
    fn read(&mut self, reg: u8, buf: &mut [u8]) -> Result<(), String>;
}

pub struct It8915<B: Bus>(pub B);

impl<B: Bus> It8915<B> {
    pub fn probe(mut bus: B) -> Result<Self, String> {
        let mut id = [0];
        match bus.read(IT8915_CHIP_REG, &mut id) {
            Ok(()) if id[0] == IT8915_CHIP_ID => {}
            Ok(()) => crate::log!("unexpected IT8915 chip id {:#04x}, relying on frame check", id[0]),
            Err(e) => crate::log!("IT8915 chip id unreadable ({e}), relying on frame check"),
        }

        let mut sensor = Self(bus);
        sensor.read()?;
        Ok(sensor)
    }

    fn frame(&mut self) -> Result<Reading, String> {
        let mut frame = [0; FRAME_LEN];
        self.0.read(IT8915_PINS_REG, &mut frame)?;
        decode(&frame)
    }
}

impl<B: Bus> Sensor for It8915<B> {
    fn read(&mut self) -> Result<Reading, String> {
        let mut result = self.frame();
        for _ in 1..ATTEMPTS {
            if result.is_ok() {
                break;
            }
            std::thread::sleep(RETRY_DELAY);
            result = self.frame();
        }

        result
    }
}

pub struct Device {
    pub name: String,
    pub pci: Option<PciId>,
    pub bus: Option<u32>,
    pub sensor: Box<dyn Sensor>,
}
