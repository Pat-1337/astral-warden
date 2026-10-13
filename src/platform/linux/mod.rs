use crate::platform::unix::{self, detach};
use gpu_warden::config::{self, Config};
use gpu_warden::display;
use gpu_warden::warden::{Host, Severity, Warden};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use unix::{Lock, spawn_quiet};

const BIN: &str = "/usr/local/bin/gpu-warden";
const RUNIT_SV: &str = "/etc/sv/gpu-warden";
const RUNSVDIRS: &[&str] = &["/var/service", "/etc/runit/runsvdir/default", "/run/runit/service"];
const MODULES: &str = "/etc/modules-load.d/gpu-warden.conf";
const UDEV: &str = "/etc/udev/rules.d/70-gpu-warden.rules";

pub fn single_instance() -> Result<Lock, String> {
    unix::lock(&[
        PathBuf::from("/run/lock/gpu-warden.lock"),
        std::env::temp_dir().join("gpu-warden.lock"),
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
        let _ = spawn_quiet("wall", &[&format!("gpu-warden: {reason}")]);

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
                "/etc/systemd/system/gpu-warden.service",
                include_str!("../../../init/gpu-warden.service"),
            ),
            Self::OpenRc => (
                "/etc/init.d/gpu-warden",
                include_str!("../../../init/gpu-warden.openrc"),
            ),
            Self::Runit => ("/etc/sv/gpu-warden/run", include_str!("../../../init/gpu-warden.runit")),
        }
    }

    fn enable(self) -> Result<(), String> {
        match self {
            Self::Systemd => spawn_quiet("systemctl", &["daemon-reload"])
                .and_then(|()| spawn_quiet("systemctl", &["enable", "gpu-warden.service"]))
                .and_then(|()| spawn_quiet("systemctl", &["restart", "gpu-warden.service"])),

            Self::OpenRc => spawn_quiet("rc-update", &["add", "gpu-warden", "default"])
                .and_then(|()| spawn_quiet("rc-service", &["gpu-warden", "restart"])),

            Self::Runit => {
                let dir = RUNSVDIRS
                    .iter()
                    .find(|d| Path::new(d).is_dir())
                    .ok_or("no runsvdir found")?;

                let link = Path::new(dir).join("gpu-warden");
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
            Self::Systemd => drop(spawn_quiet("systemctl", &["disable", "--now", "gpu-warden.service"])),

            Self::OpenRc => {
                let _ = spawn_quiet("rc-service", &["gpu-warden", "stop"]);
                let _ = spawn_quiet("rc-update", &["del", "gpu-warden", "default"]);
            }

            Self::Runit => {
                for link in RUNSVDIRS.iter().map(|d| Path::new(d).join("gpu-warden")) {
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
        PathBuf::from("/run/gpu-warden/status.json"),
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
