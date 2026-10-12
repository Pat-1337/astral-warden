use crate::dylib::Library;
use crate::hardware::PciId;
use crate::log;
use std::ffi::{CStr, c_char, c_void};
use std::path::PathBuf;

type Ret = u32;
type Device = *mut c_void;

#[cfg(windows)]
const NVML: &[&str] = &["nvml.dll", r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll"];
#[cfg(not(windows))]
const NVML: &[&str] = &["libnvidia-ml.so.1", "libnvidia-ml.so"];

struct Nvml {
    _lib: Library,
    dev: Device,
    error_string: unsafe extern "C" fn(Ret) -> *const c_char,
    get_limit: unsafe extern "C" fn(Device, *mut u32) -> Ret,
    set_limit: unsafe extern "C" fn(Device, u32) -> Ret,
    usage: unsafe extern "C" fn(Device, *mut u32) -> Ret,
    default_mw: u32,
    min_mw: u32,
    max_mw: u32,
}

unsafe impl Send for Nvml {}

#[repr(C)]
#[allow(dead_code)]
struct PciInfo {
    bus_id_legacy: [c_char; 16],
    domain: u32,
    bus: u32,
    device: u32,
    device_id: u32,
    subsystem_id: u32,
    bus_id: [c_char; 32],
}

impl Nvml {
    fn open(pci: PciId, bus: Option<u32>) -> Result<Self, String> {
        let lib = Library::open(NVML).ok_or("NVML not found")?;

        unsafe {
            macro_rules! sym {
                ($name:literal) => {
                    lib.symbol($name).ok_or(concat!("NVML lacks ", $name))?
                };
            }

            let init: unsafe extern "C" fn() -> Ret = sym!("nvmlInit_v2");
            let count: unsafe extern "C" fn(*mut u32) -> Ret = sym!("nvmlDeviceGetCount_v2");
            let by_index: unsafe extern "C" fn(u32, *mut Device) -> Ret = sym!("nvmlDeviceGetHandleByIndex_v2");
            let pci_info: unsafe extern "C" fn(Device, *mut PciInfo) -> Ret = sym!("nvmlDeviceGetPciInfo_v3");
            let constraints: unsafe extern "C" fn(Device, *mut u32, *mut u32) -> Ret =
                sym!("nvmlDeviceGetPowerManagementLimitConstraints");
            let default_limit: unsafe extern "C" fn(Device, *mut u32) -> Ret =
                sym!("nvmlDeviceGetPowerManagementDefaultLimit");

            let nvml = Self {
                error_string: sym!("nvmlErrorString"),
                get_limit: sym!("nvmlDeviceGetPowerManagementLimit"),
                set_limit: sym!("nvmlDeviceSetPowerManagementLimit"),
                usage: sym!("nvmlDeviceGetPowerUsage"),
                dev: std::ptr::null_mut(),
                default_mw: 0,
                min_mw: 0,
                max_mw: 0,
                _lib: lib,
            };

            nvml.check(init())?;
            let mut n = 0;
            nvml.check(count(&mut n))?;
            let wanted = pci.dwords();

            let dev = (0..n)
                .filter_map(|i| {
                    let mut dev = std::ptr::null_mut();
                    let mut info = std::mem::zeroed::<PciInfo>();
                    (by_index(i, &mut dev) == 0 && pci_info(dev, &mut info) == 0).then_some((dev, info))
                })
                .find(|(_, info)| (info.device_id, info.subsystem_id) == wanted && bus.is_none_or(|b| b == info.bus))
                .map(|(dev, _)| dev)
                .ok_or("NVML does not list this GPU")?;

            let (mut min, mut max, mut default) = (0, 0, 0);
            nvml.check(constraints(dev, &mut min, &mut max))?;
            nvml.check(default_limit(dev, &mut default))?;

            Ok(Self {
                dev,
                min_mw: min,
                max_mw: max,
                default_mw: default,
                ..nvml
            })
        }
    }

    fn check(&self, ret: Ret) -> Result<(), String> {
        match ret {
            0 => Ok(()),
            code => Err(unsafe { CStr::from_ptr((self.error_string)(code)) }
                .to_string_lossy()
                .into_owned()),
        }
    }

    fn query(&self, f: unsafe extern "C" fn(Device, *mut u32) -> Ret) -> Result<u32, String> {
        let mut value = 0;
        self.check(unsafe { f(self.dev, &mut value) }).map(|()| value)
    }
}

pub struct PowerCap {
    nvml: Result<Nvml, String>,
    percent: u32,
    saved_mw: Option<u32>,
    state: PathBuf,
    keep: bool,
}

impl PowerCap {
    pub fn new(pci: Option<PciId>, bus: Option<u32>, percent: u32, state: PathBuf) -> Self {
        Self {
            nvml: pci
                .ok_or_else(|| "no NVIDIA device".to_owned())
                .and_then(|pci| Nvml::open(pci, bus)),
            percent,
            saved_mw: None,
            state,
            keep: false,
        }
    }

    pub fn resume(&mut self) {
        let Some((saved, target)) = std::fs::read_to_string(&self.state).ok().and_then(|text| {
            let mut values = text.split_whitespace().map(str::parse::<u32>);
            Some((values.next()?.ok()?, values.next()?.ok()?))
        }) else {
            return;
        };

        match &self.nvml {
            Ok(nvml) if nvml.query(nvml.get_limit).is_ok_and(|current| current == target) => {
                log!(
                    "found the power cap of an earlier run, previous limit {} W",
                    saved / 1000
                );
                self.saved_mw = Some(saved);
            }
            _ => drop(std::fs::remove_file(&self.state)),
        }
    }

    pub fn available(&self) -> Result<(), &str> {
        self.nvml.as_ref().map(|_| ()).map_err(String::as_str)
    }

    pub fn watts(&self) -> Option<f32> {
        let nvml = self.nvml.as_ref().ok()?;
        nvml.query(nvml.usage).ok().map(|mw| mw as f32 / 1000.0)
    }

    pub fn limit_watts(&self) -> Option<f32> {
        let nvml = self.nvml.as_ref().ok()?;
        nvml.query(nvml.get_limit).ok().map(|mw| mw as f32 / 1000.0)
    }

    pub fn engaged(&self) -> bool {
        self.saved_mw.is_some()
    }

    pub fn keep(&mut self) {
        self.keep = true;
    }

    fn target_mw(nvml: &Nvml, percent: u32) -> u32 {
        (u64::from(nvml.default_mw) * u64::from(percent) / 100).clamp(u64::from(nvml.min_mw), u64::from(nvml.max_mw))
            as u32
    }

    pub fn engage(&mut self) -> Result<u32, String> {
        let nvml = self.nvml.as_ref().map_err(Clone::clone)?;
        let current = nvml.query(nvml.get_limit)?;
        let target = Self::target_mw(nvml, self.percent);

        if current > target {
            nvml.check(unsafe { (nvml.set_limit)(nvml.dev, target) })?;
        }

        if self.saved_mw.is_none() {
            self.saved_mw = Some(current);
            let _ = std::fs::write(&self.state, format!("{current} {}", target.min(current)));
        }

        Ok(target.min(current))
    }

    pub fn hold(&mut self) -> Result<(), String> {
        match self.saved_mw {
            Some(_) => self.engage().map(drop),
            None => Ok(()),
        }
    }

    pub fn release(&mut self) -> Result<Option<u32>, String> {
        let (Ok(nvml), Some(saved)) = (self.nvml.as_ref(), self.saved_mw) else {
            return Ok(None);
        };

        let current = nvml.query(nvml.get_limit)?;
        if current < saved {
            nvml.check(unsafe { (nvml.set_limit)(nvml.dev, saved) })?;
        }

        self.saved_mw = None;
        let _ = std::fs::remove_file(&self.state);
        Ok(Some(saved))
    }
}

impl Drop for PowerCap {
    fn drop(&mut self) {
        if !self.keep {
            let _ = self.release();
        }
    }
}
