use crate::sha256;
use gpu_warden::display;
use gpu_warden::log;
use gpu_warden::warden::{Host, Severity, Warden};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const REPO: &str = "https://github.com/patvuk/gpu-warden";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const NETWORK_WAIT: Duration = Duration::from_secs(30);
const TOOL_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(windows)]
const HELPER_TIMEOUT: Duration = Duration::from_secs(600);
#[cfg(windows)]
const HELPER_RESTART: Duration = Duration::from_secs(2);
const CURL: &[&str] = &[
    "-fsSL",
    "--proto",
    "=https",
    "--retry",
    "3",
    "--retry-connrefused",
    "--connect-timeout",
    "20",
    "--max-time",
    "240",
];

struct Asset {
    stem: &'static str,
    ext: &'static str,
    member: Option<&'static str>,
}

#[cfg(all(windows, not(feature = "portable")))]
const ASSET: Asset = Asset {
    stem: "windows-x86_64-setup",
    ext: "exe",
    member: None,
};

#[cfg(all(windows, feature = "portable"))]
const ASSET: Asset = Asset {
    stem: "windows-x86_64-portable",
    ext: "zip",
    member: Some("gpu-warden.exe"),
};

#[cfg(not(windows))]
const ASSET: Asset = Asset {
    stem: "linux-x86_64-omarchy",
    ext: "tar.gz",
    member: Some("gpu-warden"),
};

#[cfg(windows)]
const NULL: &str = "NUL";
#[cfg(windows)]
const MANAGED: &str = "Installed with Scoop. Update with: scoop update gpu-warden";

#[cfg(not(windows))]
const NULL: &str = "/dev/null";
#[cfg(not(windows))]
const MANAGED: &str = "This copy is updated by its package manager.";

static READY: AtomicBool = AtomicBool::new(false);
static CHECKING: AtomicBool = AtomicBool::new(false);
static MANUAL: AtomicBool = AtomicBool::new(false);
static NOTICED: AtomicBool = AtomicBool::new(false);
static STAGED: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);
static NOTICE: Mutex<Option<(Severity, String)>> = Mutex::new(None);
static RESTART: OnceLock<PathBuf> = OnceLock::new();
#[cfg(windows)]
static DELEGATED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static SERVING: Mutex<()> = Mutex::new(());

pub fn start(enabled: bool) {
    if enabled {
        spawn_check(false);
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn available() -> bool {
    std::env::current_exe().is_ok_and(|exe| !managed(&exe))
}

pub fn check_now() {
    spawn_check(true);
}

pub fn tick(warden: &Warden, host: &mut impl Host) -> bool {
    if NOTICED.swap(false, Ordering::Acquire)
        && let Some((severity, body)) = NOTICE.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        host.notify(severity, display::app(), &body);
    }

    #[cfg(windows)]
    if DELEGATED.load(Ordering::Acquire) {
        return true;
    }

    if !READY.load(Ordering::Acquire) || !warden.settled() {
        return false;
    }

    READY.store(false, Ordering::Relaxed);
    let Some((version, binary)) = STAGED.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return false;
    };

    match swap(&binary) {
        Ok(exe) => {
            log!("updating to {version}");
            RESTART.set(exe).is_ok()
        }
        #[cfg(windows)]
        Err(e) => {
            delegate(version, e);
            false
        }
        #[cfg(not(windows))]
        Err(e) => {
            log!("update to {version} failed: {e}");
            notice(Severity::Warn, format!("Update to {version} failed: {e}."));
            false
        }
    }
}

pub fn after_run(result: Result<(), String>, updated: bool) -> Result<(), String> {
    match (result, RESTART.get()) {
        (Ok(()), Some(exe)) => relaunch(exe, Some(VERSION)),

        (Err(e), _) if updated => {
            log!("{VERSION} failed to start, rolling back: {e}");
            undo().map_or_else(|r| Err(format!("{e}; rollback: {r}")), |exe| relaunch(&exe, None))
        }

        (result, _) => result,
    }
}

#[cfg(windows)]
fn ask_helper(line: &str) -> Result<(), String> {
    use gpu_warden::win::pipe;

    match pipe::request(pipe::POWER, line.to_owned(), HELPER_TIMEOUT)?.as_str() {
        "ok" => Ok(()),
        reply => Err(reply.trim_start_matches("error: ").to_owned()),
    }
}

#[cfg(windows)]
fn delegate(version: String, local: String) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };

    std::thread::spawn(move || match ask_helper(&format!("update {version}")) {
        Ok(()) => {
            log!("updating to {version} through the power helper");
            if RESTART.set(exe).is_ok() {
                DELEGATED.store(true, Ordering::Release);
            }
        }
        Err(e) => {
            log!("update to {version} failed: {local}; power helper: {e}");
            notice(Severity::Warn, format!("Update to {version} failed: {e}."));
        }
    });
}

#[cfg(windows)]
fn undo() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    rollback().or_else(|local| {
        ask_helper("rollback")
            .map(|()| exe)
            .map_err(|e| format!("{local}; power helper: {e}"))
    })
}

#[cfg(not(windows))]
fn undo() -> Result<PathBuf, String> {
    rollback()
}

#[cfg(windows)]
fn release(tag: &str) -> Result<PathBuf, String> {
    let canonical = parse(tag).map(|(major, minor, patch)| format!("v{major}.{minor}.{patch}"));
    let newer = matches!((parse(tag), parse(VERSION)), (Some(new), Some(current)) if new > current);

    if !newer || canonical.as_deref() != Some(tag) {
        return Err(format!("{tag:?} is not a newer release"));
    }

    let binary = stage(tag)?;
    swap(&binary)
}

#[cfg(windows)]
fn restart_helper(exe: PathBuf) {
    std::thread::spawn(move || {
        std::thread::sleep(HELPER_RESTART);

        let mut cmd = Command::new(&exe);
        cmd.arg("power-helper");
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);

        match cmd.spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => log!("power helper restart: {}: {e}", exe.display()),
        }
    });
}

#[cfg(windows)]
pub fn serve(line: &str) -> Option<String> {
    let busy = SERVING.lock().unwrap_or_else(|e| e.into_inner());

    let result = match line.split_once(' ') {
        Some(("update", tag)) => release(tag),
        None if line == "rollback" => rollback(),
        _ => return None,
    };
    drop(busy);

    Some(match result {
        Ok(exe) => {
            log!("{line} done for an unelevated instance");
            restart_helper(exe);
            "ok".into()
        }
        Err(e) => {
            log!("{line} failed: {e}");
            format!("error: {e}")
        }
    })
}

fn notice(severity: Severity, body: String) {
    log!("[{severity:?}] {body}");
    *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) = Some((severity, body));
    NOTICED.store(true, Ordering::Release);
}

fn spawn_check(manual: bool) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };

    if managed(&exe) {
        match manual {
            true => notice(Severity::Info, MANAGED.into()),
            false => log!("auto update off: {} is not a standalone install", exe.display()),
        }
        return;
    }

    if READY.load(Ordering::Acquire) {
        if manual {
            notice(
                Severity::Info,
                "An update is ready. It installs once all pins read OK.".into(),
            );
        }
        return;
    }

    if manual {
        MANUAL.store(true, Ordering::Release);
    }

    if CHECKING.swap(true, Ordering::AcqRel) {
        return;
    }

    let _ = fs::remove_file(sibling(&exe, "failed"));
    let skip = match manual {
        true => None,
        false => fs::read_to_string(skip_file(&exe)).ok(),
    };

    std::thread::spawn(move || {
        let second = Duration::from_secs(1);
        for _ in 0..NETWORK_WAIT.as_secs() {
            if MANUAL.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(second);
        }

        let manual = MANUAL.load(Ordering::Acquire);
        let skip = skip.filter(|_| !manual);

        match check(skip.as_deref()) {
            Ok(Some((tag, binary))) => {
                log!("update {tag} staged, applying when all pins read OK");
                if manual {
                    notice(Severity::Info, format!("Update {tag} found. Installing it now."));
                }

                *STAGED.lock().unwrap_or_else(|e| e.into_inner()) = Some((tag, binary));
                READY.store(true, Ordering::Release);
            }

            Ok(None) if manual => notice(Severity::Info, format!("Up to date ({VERSION}).")),
            Ok(None) => {}

            Err(e) => {
                log!("update check failed: {e}");
                if manual {
                    notice(Severity::Warn, format!("Update check failed: {e}."));
                }
            }
        }

        MANUAL.store(false, Ordering::Release);
        CHECKING.store(false, Ordering::Release);
    });
}

fn managed(exe: &Path) -> bool {
    #[cfg(windows)]
    return exe.components().any(|c| c.as_os_str().eq_ignore_ascii_case("scoop"));

    #[cfg(not(windows))]
    return exe != Path::new("/usr/local/bin/gpu-warden");
}

fn sibling(exe: &Path, tag: &str) -> PathBuf {
    match exe.extension() {
        Some(ext) => exe.with_extension(format!("{tag}.{}", ext.to_string_lossy())),
        None => exe.with_extension(tag),
    }
}

fn skip_file(exe: &Path) -> PathBuf {
    exe.with_extension("skip")
}

fn work_dir() -> PathBuf {
    #[cfg(windows)]
    return std::env::temp_dir().join("gpu-warden-update");

    #[cfg(not(windows))]
    return PathBuf::from("/var/lib/gpu-warden/update");
}

fn system_tool(program: &str) -> PathBuf {
    #[cfg(windows)]
    return std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("System32")
        .join(format!("{program}.exe"));

    #[cfg(not(windows))]
    return PathBuf::from(program);
}

fn tool(program: &str, args: &[&str]) -> Result<String, String> {
    run(&system_tool(program), args)
}

fn run(program: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);

    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", program.display()))?;
    let deadline = std::time::Instant::now() + TOOL_TIMEOUT;
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{}: timed out", program.display()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();

    match out.status.success() {
        true => Ok(text),
        false => Err(format!(
            "{}: {}",
            program.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

fn parse(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim_start_matches('v').split('.').map(|p| p.parse().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

fn curl(args: &[&str]) -> Result<String, String> {
    tool("curl", &[CURL, args].concat())
}

fn check(skip: Option<&str>) -> Result<Option<(String, PathBuf)>, String> {
    let latest = format!("{REPO}/releases/latest");
    let url = curl(&["-I", "-o", NULL, "-w", "%{url_effective}", &latest])?;
    let tag = url.rsplit('/').next().unwrap_or_default().to_owned();

    if skip.is_some_and(|s| s.trim() == tag.trim_start_matches('v')) {
        log!("skipping {tag}, it failed to start before");
        return Ok(None);
    }

    match (parse(&tag), parse(VERSION)) {
        (Some(new), Some(current)) if new > current => stage(&tag).map(|binary| Some((tag, binary))),
        (Some(_), Some(_)) => Ok(None),
        _ => Err(format!("unexpected release url {url}")),
    }
}

fn stage(tag: &str) -> Result<PathBuf, String> {
    let dir = work_dir();
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let name = format!("gpu-warden-{tag}-{}", ASSET.stem);
    let file = format!("{name}.{}", ASSET.ext);
    let (download_path, sums) = (dir.join(&file), dir.join("SHA256SUMS"));
    download(tag, &file, &download_path)?;
    download(tag, "SHA256SUMS", &sums)?;

    let expected = fs::read_to_string(&sums)
        .map_err(|e| format!("SHA256SUMS: {e}"))?
        .lines()
        .find_map(|line| line.strip_suffix(&format!("  {file}")).map(str::to_owned))
        .ok_or_else(|| format!("{file} missing from SHA256SUMS"))?;

    let bytes = fs::read(&download_path).map_err(|e| format!("{}: {e}", download_path.display()))?;
    if sha256::hex(&bytes) != expected.to_ascii_lowercase() {
        return Err("checksum mismatch".into());
    }

    let binary = match ASSET.member {
        None => download_path,
        Some(member) => {
            let (archive, dir_arg) = (download_path.to_string_lossy(), dir.to_string_lossy());
            tool("tar", &["-xf", &archive, "-C", &dir_arg, &format!("{name}/{member}")])?;
            dir.join(&name).join(member)
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&binary, fs::Permissions::from_mode(0o755));
    }

    let version = run(&binary, &["--version"])?;
    if version != format!("gpu-warden {}", tag.trim_start_matches('v')) {
        return Err(format!("staged binary reports {version:?}"));
    }

    Ok(binary)
}

fn download(tag: &str, asset: &str, to: &Path) -> Result<(), String> {
    let url = format!("{REPO}/releases/download/{tag}/{asset}");
    let to = to.to_string_lossy();
    curl(&["-o", &to, &url]).map(drop)
}

fn swap(binary: &Path) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (new, old) = (sibling(&exe, "new"), sibling(&exe, "old"));

    fs::copy(binary, &new).map_err(|e| format!("{}: {e}", new.display()))?;
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|e| format!("{}: {e}", exe.display()))?;

    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(format!("{}: {e}", exe.display()));
    }

    let _ = fs::remove_dir_all(work_dir());
    let _ = fs::remove_file(skip_file(&exe));
    Ok(exe)
}

fn rollback() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (old, failed) = (sibling(&exe, "old"), sibling(&exe, "failed"));

    if !old.exists() {
        return Err(format!("{} missing", old.display()));
    }

    fs::rename(&exe, &failed)
        .and_then(|()| fs::rename(&old, &exe))
        .map_err(|e| format!("{}: {e}", exe.display()))?;

    let _ = fs::write(skip_file(&exe), VERSION);
    Ok(exe)
}

fn relaunch(exe: &Path, updated_from: Option<&str>) -> Result<(), String> {
    let mut args: Vec<String> = Vec::new();
    let mut given = std::env::args().skip(1);
    while let Some(arg) = given.next() {
        match arg.as_str() {
            "--updated" => drop(given.next()),
            _ => args.push(arg),
        }
    }

    if let Some(version) = updated_from {
        args.extend(["--updated".into(), version.into()]);
    }

    #[cfg(windows)]
    {
        if !args.iter().any(|a| a == "--silent") {
            args.push("--silent".into());
        }

        Command::new(exe)
            .args(&args)
            .spawn()
            .map(drop)
            .map_err(|e| format!("{}: {e}", exe.display()))
    }

    #[cfg(not(windows))]
    {
        use std::os::unix::process::CommandExt;
        Err(format!("{}: {}", exe.display(), Command::new(exe).args(&args).exec()))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn runs_absolute_paths() {
        assert_eq!(
            run(Path::new("/bin/echo"), &["gpu-warden 1.2.3"]).unwrap(),
            "gpu-warden 1.2.3"
        );
        assert!(run(Path::new("/bin/false"), &[]).is_err());
    }
}
