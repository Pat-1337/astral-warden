use super::unix;
use crate::config::Config;
use crate::hardware::Device;
use crate::log;
use crate::warden::{Host, Severity, Warden};
use std::path::Path;

pub use unix::Lock;

struct Console;

impl Host for Console {
    fn notify(&mut self, _: Severity, _: &str, _: &str) {}

    fn alarm(&mut self, severity: Severity) {
        log!("alarm ({severity:?})");
    }

    fn shutdown(&mut self, reason: &str) -> Result<(), String> {
        log!("unsupported platform, not shutting down: {reason}");
        Ok(())
    }
}

pub fn migrate(_: Option<&str>) -> bool {
    false
}

pub fn open_device(_: &Config) -> Result<Device, String> {
    Err("hardware access needs Windows or Linux; set `simulate` in the config to dry-run".into())
}

pub fn single_instance() -> Result<Lock, String> {
    unix::lock(&[std::env::temp_dir().join("gpu-warden.lock")])
}

pub fn run(warden: Warden, cfg: &Config, _: &Path, _: bool) -> Result<(), String> {
    unix::run_loop(warden, cfg.poll(), &mut Console, |_, _| {})
}

pub fn install(_: &Path, _: bool) -> Result<(), String> {
    Err("install is supported on Windows and Linux only".into())
}

pub fn uninstall(_: bool) -> Result<(), String> {
    install(Path::new(""), false)
}

pub fn memory() -> Option<(f64, f64)> {
    None
}

pub fn attach_console() {}

pub fn fatal(_: &str) {}
