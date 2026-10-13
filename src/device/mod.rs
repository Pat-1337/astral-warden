#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

use crate::hardware::Device;
use crate::sensor::Simulated;
use std::path::Path;

pub fn open(wireview_port: Option<&Path>) -> Result<Device, String> {
    #[cfg(any(target_os = "linux", windows))]
    return os::open(wireview_port);

    #[cfg(not(any(target_os = "linux", windows)))]
    return Err(format!(
        "hardware access needs Windows or Linux ({wireview_port:?}); set `simulate` to dry-run"
    ));
}

pub fn simulated(path: &Path) -> Device {
    Device {
        name: "Simulated".into(),
        detail: path.display().to_string(),
        pci: None,
        bus: None,
        sensor: Box::new(Simulated(path.to_owned())),
    }
}
