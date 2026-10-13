use crate::config::{Config, Limits};
use crate::guard::{self, Fault, Level};
use crate::hardware::Device;
use crate::power::PowerCap;
use crate::sensor::{Reading, Sensor};
use crate::{device, log};
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;

pub struct GwDevice {
    sensor: Box<dyn Sensor>,
    cap: PowerCap,
    name: CString,
    detail: CString,
}

#[repr(C)]
pub struct GwLimits {
    pin_warn: f32,
    pin_crit: f32,
    imbalance_warn: f32,
    imbalance_crit: f32,
    imbalance_min_total_amps: f32,
}

thread_local! {
    static ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn text(value: String) -> CString {
    CString::new(value.replace('\0', "")).unwrap_or_default()
}

fn fail<T>(error: String, value: T) -> T {
    ERROR.with(|slot| *slot.borrow_mut() = text(error));
    value
}

unsafe fn path(ptr: *const c_char) -> Option<PathBuf> {
    match ptr.is_null() {
        true => None,
        false => Some(PathBuf::from(
            unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned(),
        )),
    }
}

fn wrap(opened: Result<Device, String>) -> *mut GwDevice {
    let device = match opened {
        Ok(device) => device,
        Err(e) => return fail(e, std::ptr::null_mut()),
    };

    let state = std::env::temp_dir().join("gpu-warden-sdk.power-cap");
    Box::into_raw(Box::new(GwDevice {
        cap: PowerCap::new(device.pci, device.bus, 100, state),
        name: text(device.name),
        detail: text(device.detail),
        sensor: device.sensor,
    }))
}

fn config(limits: Option<&GwLimits>) -> Config {
    let mut cfg = Config::default();

    if let Some(l) = limits {
        cfg.pin_amps = Limits {
            warn: l.pin_warn,
            crit: l.pin_crit,
        };
        cfg.imbalance = Limits {
            warn: l.imbalance_warn,
            crit: l.imbalance_crit,
        };
        cfg.imbalance_min_total_amps = l.imbalance_min_total_amps;
    }

    cfg
}

#[unsafe(no_mangle)]
pub extern "C" fn gw_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn gw_last_error() -> *const c_char {
    ERROR.with(|slot| slot.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_open(wireview_port: *const c_char) -> *mut GwDevice {
    log::quiet();
    let port = unsafe { path(wireview_port) };
    wrap(device::open(port.as_deref()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_open_simulated(file: *const c_char) -> *mut GwDevice {
    log::quiet();
    match unsafe { path(file) } {
        Some(file) => wrap(Ok(device::simulated(&file))),
        None => fail("no file given".into(), std::ptr::null_mut()),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_close(device: *mut GwDevice) {
    if !device.is_null() {
        drop(unsafe { Box::from_raw(device) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_name(device: *const GwDevice) -> *const c_char {
    unsafe { &*device }.name.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_detail(device: *const GwDevice) -> *const c_char {
    unsafe { &*device }.detail.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_read(device: *mut GwDevice, out: *mut Reading) -> bool {
    match unsafe { &mut *device }.sensor.read() {
        Ok(reading) => {
            unsafe { out.write(reading) };
            true
        }
        Err(e) => fail(e, false),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_assess(reading: *const Reading, limits: *const GwLimits, pin: *mut u32) -> i32 {
    let verdict = guard::assess(unsafe { &*reading }, &config(unsafe { limits.as_ref() }));

    let (level, at) = match verdict {
        None => return 0,
        Some((level, Fault::Pin { pin, .. } | Fault::Imbalance { pin, .. })) => (level, pin),
    };

    if !pin.is_null() {
        unsafe { pin.write(at as u32) };
    }

    match level {
        Level::Warn => 1,
        Level::Crit => 2,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_imbalance(reading: *const Reading, pin: *mut u32) -> f32 {
    let (at, deviation) = guard::imbalance(unsafe { &*reading });

    if !pin.is_null() {
        unsafe { pin.write(at as u32) };
    }

    deviation
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_board_watts(device: *const GwDevice) -> f32 {
    unsafe { &*device }.cap.watts().unwrap_or(f32::NAN)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_limit_watts(device: *const GwDevice) -> f32 {
    unsafe { &*device }.cap.limit_watts().unwrap_or(f32::NAN)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_cap(device: *mut GwDevice, percent: u32) -> bool {
    let cap = &mut unsafe { &mut *device }.cap;
    cap.set_percent(percent);

    match cap.engage() {
        Ok(_) => true,
        Err(e) => fail(e, false),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gw_release(device: *mut GwDevice) -> bool {
    match unsafe { &mut *device }.cap.release() {
        Ok(_) => true,
        Err(e) => fail(e, false),
    }
}
