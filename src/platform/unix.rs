use crate::warden::{Host, Warden};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static STOP: AtomicBool = AtomicBool::new(false);

pub struct Lock(#[allow(dead_code)] File);

pub fn lock(path: &Path) -> Result<Lock, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    match unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } {
        0 => Ok(Lock(file)),
        _ => Err("another instance is already running".into()),
    }
}

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn run_loop(mut warden: Warden, poll: Duration, host: &mut impl Host) -> Result<(), String> {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        unsafe { libc::signal(signal, on_signal as *const () as libc::sighandler_t) };
    }
    let mut next = Instant::now();
    while !STOP.load(Ordering::Relaxed) {
        if warden.tick(host).is_some() {
            break;
        }
        next += poll;
        let now = Instant::now();
        match next.checked_duration_since(now) {
            Some(wait) => std::thread::sleep(wait),
            None => next = now,
        }
    }
    Ok(())
}

pub fn spawn_quiet(program: &str, args: &[&str]) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("{program}: {e}"))
        .and_then(|s| {
            if s.success() {
                Ok(())
            } else {
                Err(format!("{program}: {s}"))
            }
        })
}

pub fn detach(cmd: &mut std::process::Command) {
    let spawned = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = spawned {
        std::thread::spawn(move || child.wait());
    }
}
