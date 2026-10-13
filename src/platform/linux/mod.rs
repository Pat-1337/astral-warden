mod wireview;

use crate::config::{self, Config};
use crate::display;
use crate::hardware::{Bus, Device, Gpu, IT8915_ADDR, It8915, NVIDIA, PciId, inline_device, no_device};
use crate::platform::unix::{self, detach};
use crate::warden::{Host, Severity, Warden};
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use unix::{Lock, spawn_quiet};

const PCI_DEVICES: &str = "/sys/bus/pci/devices";
const ADAPTER_PREFIX: &str = "NVIDIA i2c adapter ";
const SENSOR_ADAPTER: &str = "1";
const I2C_SLAVE: u64 = 0x0703;
const I2C_SMBUS: u64 = 0x0720;
const I2C_SMBUS_READ: u8 = 1;
const I2C_SMBUS_I2C_BLOCK_DATA: u32 = 8;
const BIN: &str = "/usr/local/bin/astral-warden";
const RUNIT_SV: &str = "/etc/sv/astral-warden";
const RUNSVDIRS: &[&str] = &["/var/service", "/etc/runit/runsvdir/default", "/run/runit/service"];
const MODULES: &str = "/etc/modules-load.d/astral-warden.conf";
const UDEV: &str = "/etc/udev/rules.d/70-astral-warden.rules";

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

pub fn open_device(cfg: &Config) -> Result<Device, String> {
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
        if cfg.wireview_port.is_some() {
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

    match wireview::open(cfg.wireview_port.as_deref()) {
        Some(Ok((sensor, model, via))) => Ok(inline_device(sensor, model, via, &gpus)),
        Some(Err(e)) => {
            failures.push(format!("WireView Pro II: {e}"));
            Err(no_device(&failures))
        }
        None => Err(no_device(&failures)),
    }
}

pub fn single_instance() -> Result<Lock, String> {
    unix::lock(&[
        PathBuf::from("/run/lock/astral-warden.lock"),
        std::env::temp_dir().join("astral-warden.lock"),
    ])
}

struct Desktop;

impl Desktop {
    fn sessions() -> Vec<(String, PathBuf)> {
        fs::read_dir("/run/user")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let uid = entry.file_name().to_str()?.parse().ok()?;
                let bus = entry.path().join("bus");
                Some((user_name(uid)?, bus.exists().then_some(bus)?))
            })
            .collect()
    }
}

pub fn user_name(uid: libc::uid_t) -> Option<String> {
    let mut pwd = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut buf = [0 as libc::c_char; 1024];
    let mut out = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut out) };

    (rc == 0 && !out.is_null()).then(|| {
        unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }
            .to_string_lossy()
            .into_owned()
    })
}

impl Desktop {
    fn in_sessions(program: &str, args: &[&str]) {
        if unsafe { libc::geteuid() } != 0 {
            return detach(Command::new(program).args(args));
        }

        for (user, bus) in Self::sessions() {
            let bus_env = format!("DBUS_SESSION_BUS_ADDRESS=unix:path={}", bus.display());
            let runtime = format!("XDG_RUNTIME_DIR={}", bus.parent().unwrap_or(&bus).display());
            detach(
                Command::new("runuser")
                    .args(["-u", &user, "--", "env", &bus_env, &runtime, program])
                    .args(args),
            );
        }
    }
}

impl Host for Desktop {
    fn notify(&mut self, severity: Severity, title: &str, body: &str) {
        let (urgency, icon) = match severity {
            Severity::Info => ("normal", "dialog-information"),
            Severity::Warn => ("critical", "dialog-warning"),
            Severity::Crit => ("critical", "dialog-error"),
        };

        Self::in_sessions(
            "notify-send",
            &["-a", display::app(), "-i", icon, "-u", urgency, title, body],
        );
    }

    fn alarm(&mut self, severity: Severity) {
        let sound = match severity {
            Severity::Crit => "alarm-clock-elapsed",
            Severity::Info | Severity::Warn => "dialog-warning",
        };

        Self::in_sessions("canberra-gtk-play", &["-i", sound]);
        let _ = fs::write("/dev/console", b"\x07");
    }

    fn shutdown(&mut self, reason: &str) -> Result<(), String> {
        let _ = spawn_quiet("wall", &[&format!("astral-warden: {reason}")]);

        spawn_quiet("systemctl", &["poweroff"])
            .or_else(|_| spawn_quiet("shutdown", &["-P", "now"]))
            .or_else(|_| spawn_quiet("poweroff", &[]))
            .or_else(|e| {
                unsafe { libc::sync() };
                match unsafe { libc::reboot(libc::RB_POWER_OFF) } {
                    0 => Ok(()),
                    _ => Err(format!("{e}; reboot(2): {}", std::io::Error::last_os_error())),
                }
            })
    }
}

pub fn run(warden: Warden, cfg: &Config, _: &Path, _: bool) -> Result<(), String> {
    #[cfg(feature = "omarchy")]
    let on_tick = {
        let mut publisher = crate::omarchy::Publisher::new();
        move |w: &Warden, host: &mut Desktop| {
            publisher.tick(w);

            if crate::update::tick(w, host) {
                unix::stop();
            }
        }
    };

    #[cfg(not(feature = "omarchy"))]
    let on_tick = |_: &Warden, _: &mut Desktop| {};

    unix::run_loop(warden, cfg.poll(), &mut Desktop, on_tick)
}

#[derive(Debug, Clone, Copy)]
enum Init {
    Systemd,
    OpenRc,
    Runit,
}

impl Init {
    fn detect() -> Result<Self, String> {
        let exists = |p: &str| Path::new(p).exists();
        match () {
            _ if exists("/run/systemd/system") => Ok(Self::Systemd),
            _ if exists("/run/openrc") || exists("/sbin/openrc-run") => Ok(Self::OpenRc),
            _ if exists("/run/runit") || exists("/etc/runit") => Ok(Self::Runit),
            _ => Err(format!(
                "unknown init system; start `{BIN} run` as root at boot yourself"
            )),
        }
    }

    fn service(self) -> (&'static str, &'static str) {
        match self {
            Self::Systemd => (
                "/etc/systemd/system/astral-warden.service",
                include_str!("../../../init/astral-warden.service"),
            ),
            Self::OpenRc => (
                "/etc/init.d/astral-warden",
                include_str!("../../../init/astral-warden.openrc"),
            ),
            Self::Runit => (
                "/etc/sv/astral-warden/run",
                include_str!("../../../init/astral-warden.runit"),
            ),
        }
    }

    fn enable(self) -> Result<(), String> {
        match self {
            Self::Systemd => spawn_quiet("systemctl", &["daemon-reload"])
                .and_then(|()| spawn_quiet("systemctl", &["enable", "astral-warden.service"]))
                .and_then(|()| spawn_quiet("systemctl", &["restart", "astral-warden.service"])),

            Self::OpenRc => spawn_quiet("rc-update", &["add", "astral-warden", "default"])
                .and_then(|()| spawn_quiet("rc-service", &["astral-warden", "restart"])),

            Self::Runit => {
                let dir = RUNSVDIRS
                    .iter()
                    .find(|d| Path::new(d).is_dir())
                    .ok_or("no runsvdir found")?;

                let link = Path::new(dir).join("astral-warden");
                match std::os::unix::fs::symlink(RUNIT_SV, &link) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                        spawn_quiet("sv", &["restart", &link.to_string_lossy()])
                    }
                    Err(e) => Err(format!("{}: {e}", link.display())),
                }
            }
        }
    }

    fn disable(self) {
        match self {
            Self::Systemd => drop(spawn_quiet("systemctl", &["disable", "--now", "astral-warden.service"])),

            Self::OpenRc => {
                let _ = spawn_quiet("rc-service", &["astral-warden", "stop"]);
                let _ = spawn_quiet("rc-update", &["del", "astral-warden", "default"]);
            }

            Self::Runit => {
                for link in RUNSVDIRS.iter().map(|d| Path::new(d).join("astral-warden")) {
                    let _ = spawn_quiet("sv", &["down", &link.to_string_lossy()]);
                    let _ = fs::remove_file(link);
                }

                let _ = fs::remove_dir_all(RUNIT_SV);
            }
        }
    }
}

fn write(path: &str, text: &str, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(dir) = Path::new(path).parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }

    fs::write(path, text)
        .and_then(|()| fs::set_permissions(path, fs::Permissions::from_mode(mode)))
        .map_err(|e| format!("{path}: {e}"))
}

fn require_root() -> Result<(), String> {
    match unsafe { libc::geteuid() } {
        0 => Ok(()),
        _ => Err("run with sudo".into()),
    }
}

fn remove(path: &str) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(format!("{path}: {e}")),
        _ => Ok(()),
    }
}

pub fn install(config: &Path, _: bool) -> Result<(), String> {
    require_root()?;
    let init = Init::detect()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    if exe != Path::new(BIN) {
        let staged = format!("{BIN}.new");
        fs::copy(&exe, &staged)
            .and_then(|_| fs::rename(&staged, BIN))
            .map_err(|e| format!("{BIN}: {e}"))?;
    }

    if Config::write_default(config).map_err(|e| format!("{}: {e}", config.display()))? {
        println!("wrote {}", config.display());
    }

    write(MODULES, include_str!("../../../init/modules-load.conf"), 0o644)?;
    let _ = spawn_quiet("modprobe", &["i2c-dev"]);

    write(UDEV, include_str!("../../../init/udev.rules"), 0o644)?;
    let _ = spawn_quiet("udevadm", &["control", "--reload"]);

    let (path, template) = init.service();
    let service = template
        .replace("@BIN@", BIN)
        .replace("@CONFIG@", &config.to_string_lossy());
    write(
        path,
        &service,
        if matches!(init, Init::Systemd) { 0o644 } else { 0o755 },
    )?;

    init.enable()
        .map_err(|e| format!("{path} written, but starting it failed: {e}"))?;

    println!("installed as a {init:?} service: {path}");

    #[cfg(feature = "omarchy")]
    crate::omarchy::for_sudo_user(BIN, "install");

    Ok(())
}

pub fn uninstall(_: bool) -> Result<(), String> {
    require_root()?;
    let init = Init::detect()?;

    #[cfg(feature = "omarchy")]
    crate::omarchy::for_sudo_user(BIN, "uninstall");

    init.disable();
    remove(init.service().0)?;
    remove(MODULES)?;
    remove(UDEV)?;
    let _ = spawn_quiet("udevadm", &["control", "--reload"]);

    for suffix in ["", ".old", ".new", ".failed", ".skip"] {
        remove(&format!("{BIN}{suffix}"))?;
    }

    remove(&config::state_path().to_string_lossy())?;
    for dir in [
        config::default_path(),
        config::note_path(),
        PathBuf::from("/run/astral-warden/status.json"),
    ] {
        if let Some(dir) = dir.parent() {
            let _ = fs::remove_dir_all(dir);
        }
    }

    if matches!(init, Init::Systemd) {
        let _ = spawn_quiet("systemctl", &["daemon-reload"]);
    }

    println!("uninstalled; nothing is left behind");
    Ok(())
}

pub fn memory() -> Option<(f64, f64)> {
    let text = fs::read_to_string("/proc/self/statm").ok()?;
    let mut pages = text.split_whitespace().skip(1).map(|f| f.parse::<f64>().ok());
    let (resident, shared) = (pages.next()??, pages.next()??);

    let mb = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as f64 / (1024.0 * 1024.0);
    Some((resident * mb, (resident - shared) * mb))
}

pub fn attach_console() {}

pub fn fatal(_: &str) {}
