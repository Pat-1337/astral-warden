mod wireview;

use crate::hardware::{Bus, Device, Gpu, IT8915_ADDR, It8915, NVIDIA, PciId, inline_device, no_device};
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::path::Path;

const PCI_DEVICES: &str = "/sys/bus/pci/devices";
const ADAPTER_PREFIX: &str = "NVIDIA i2c adapter ";
const SENSOR_ADAPTER: &str = "1";
const I2C_SLAVE: u64 = 0x0703;
const I2C_SMBUS: u64 = 0x0720;
const I2C_SMBUS_READ: u8 = 1;
const I2C_SMBUS_I2C_BLOCK_DATA: u32 = 8;

#[repr(C)]
struct SmbusIoctl {
    read_write: u8,
    command: u8,
    size: u32,
    data: *mut [u8; 34],
}

struct I2cDev(File);

impl I2cDev {
    fn open(adapter: u32) -> Result<Self, String> {
        let path = format!("/dev/i2c-{adapter}");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| match e.kind() {
                ErrorKind::NotFound => format!("{path} missing, run `modprobe i2c-dev`"),
                ErrorKind::PermissionDenied => format!("{path} needs root"),
                _ => format!("{path}: {e}"),
            })?;

        match unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE as _, libc::c_ulong::from(IT8915_ADDR)) } {
            0 => Ok(Self(file)),
            _ => Err(format!(
                "{path}: address {IT8915_ADDR:#04x}: {}",
                std::io::Error::last_os_error()
            )),
        }
    }
}

impl Bus for I2cDev {
    fn read(&mut self, reg: u8, buf: &mut [u8]) -> Result<(), String> {
        let mut block = [0u8; 34];
        block[0] = buf.len() as u8;

        let mut args = SmbusIoctl {
            read_write: I2C_SMBUS_READ,
            command: reg,
            size: I2C_SMBUS_I2C_BLOCK_DATA,
            data: &mut block,
        };

        match unsafe { libc::ioctl(self.0.as_raw_fd(), I2C_SMBUS as _, &mut args) } {
            0 => {
                buf.copy_from_slice(&block[1..=buf.len()]);
                Ok(())
            }
            _ => Err(format!("i2c read {reg:#04x}: {}", std::io::Error::last_os_error())),
        }
    }
}

fn read_hex(dir: &Path, file: &str) -> Option<u16> {
    let text = fs::read_to_string(dir.join(file)).ok()?;
    u16::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}

fn pci_id(dir: &Path) -> Option<PciId> {
    Some(PciId {
        vendor: read_hex(dir, "vendor")?,
        device: read_hex(dir, "device")?,
        sub_vendor: read_hex(dir, "subsystem_vendor")?,
        sub_device: read_hex(dir, "subsystem_device")?,
    })
}

fn sensor_adapter(gpu: &Path) -> Result<u32, String> {
    fs::read_dir(gpu)
        .map_err(|e| format!("{}: {e}", gpu.display()))?
        .flatten()
        .filter_map(|entry| {
            let index = entry.file_name().to_str()?.strip_prefix("i2c-")?.parse().ok()?;
            let name = fs::read_to_string(entry.path().join("name")).ok()?;
            let port = name.strip_prefix(ADAPTER_PREFIX)?.split_whitespace().next()?;
            (port == SENSOR_ADAPTER).then_some(index)
        })
        .min()
        .ok_or_else(|| "NVIDIA i2c adapter 1 not found (needs the NVIDIA driver)".into())
}

fn is_display(dir: &Path) -> bool {
    fs::read_to_string(dir.join("class")).is_ok_and(|class| class.starts_with("0x03"))
}

pub fn open(wireview_port: Option<&Path>) -> Result<Device, String> {
    let mut failures = Vec::new();
    let mut gpus = Vec::new();

    for dir in fs::read_dir(PCI_DEVICES)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
    {
        let Some(pci) = pci_id(&dir).filter(|id| id.vendor == NVIDIA && is_display(&dir)) else {
            continue;
        };

        let bdf = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let bus = bdf.split(':').nth(1).and_then(|b| u32::from_str_radix(b, 16).ok());
        gpus.push(Gpu { pci, bus });
        if wireview_port.is_some() {
            continue;
        }

        let Some(model) = pci.model() else {
            failures.push(format!("{pci} is not a supported board"));
            continue;
        };

        match sensor_adapter(&dir).and_then(|n| Ok((n, It8915::probe(I2cDev::open(n)?)?))) {
            Ok((n, sensor)) => {
                return Ok(Device {
                    name: model,
                    detail: format!("[{pci}] at {bdf} via /dev/i2c-{n}"),
                    pci: Some(pci),
                    bus,
                    sensor: Box::new(sensor),
                });
            }
            Err(e) => failures.push(format!("{model} at {bdf}: {e}")),
        }
    }

    match wireview::open(wireview_port) {
        Some(Ok((sensor, model, via))) => Ok(inline_device(sensor, model, via, &gpus)),
        Some(Err(e)) => {
            failures.push(format!("WireView Pro II: {e}"));
            Err(no_device(&failures))
        }
        None => Err(no_device(&failures)),
    }
}
