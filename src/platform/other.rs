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

    fn shutdown(&mut self, reason: &str) -> Result<(), String> {
        log!("unsupported platform, not shutting down: {reason}");
        Ok(())
    }
}

pub fn open_device() -> Result<Device, String> {
    Err("hardware access needs Windows or Linux; set `simulate` in the config to dry-run".into())
}

pub fn single_instance() -> Result<Lock, String> {
    unix::lock(&std::env::temp_dir().join("astral-warden.lock"))
}

pub fn run(warden: Warden, cfg: &Config) -> Result<(), String> {
    unix::run_loop(warden, cfg.poll(), &mut Console)
}

pub fn install(_: &Path) -> Result<(), String> {
    Err("install is supported on Windows and Linux only".into())
}

pub fn uninstall() -> Result<(), String> {
    install(Path::new(""))
}

pub fn attach_console() {}

pub fn fatal(_: &str) {}
