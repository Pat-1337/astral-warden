use crate::log;
use crate::sha256;
use crate::warden::Warden;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const REPO: &str = "https://github.com/Pat-1337/astral-warden";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const NETWORK_WAIT: Duration = Duration::from_secs(30);
const CURL: &[&str] = &["-fsSL", "--proto", "=https", "--retry", "3", "--retry-connrefused"];

#[cfg(windows)]
const FLAVOR: &str = "windows-x86_64";
#[cfg(windows)]
const ARCHIVE: &str = "zip";
#[cfg(windows)]
const MEMBER: &str = "astral-warden.exe";
#[cfg(windows)]
const NULL: &str = "NUL";

#[cfg(not(windows))]
const FLAVOR: &str = "linux-x86_64-omarchy";
#[cfg(not(windows))]
const ARCHIVE: &str = "tar.gz";
#[cfg(not(windows))]
const MEMBER: &str = "astral-warden";
#[cfg(not(windows))]
const NULL: &str = "/dev/null";

static READY: AtomicBool = AtomicBool::new(false);
static STAGED: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);
static RESTART: OnceLock<PathBuf> = OnceLock::new();

pub fn start(enabled: bool) {
    if !enabled {
        return;
    }

    let Ok(exe) = std::env::current_exe() else {
        return;
    };

    if managed(&exe) {
        log!("auto update off: {} is not a standalone install", exe.display());
        return;
    }

    let _ = fs::remove_file(sibling(&exe, "failed"));
    let skip = fs::read_to_string(skip_file(&exe)).ok();

    std::thread::spawn(move || {
        std::thread::sleep(NETWORK_WAIT);

        match check(skip.as_deref()) {
            Ok(Some(staged)) => {
                log!("update {} staged, applying when all pins read OK", staged.0);
                *STAGED.lock().unwrap_or_else(|e| e.into_inner()) = Some(staged);
                READY.store(true, Ordering::Release);
            }
            Ok(None) => {}
            Err(e) => log!("update check failed: {e}"),
        }
    });
}

pub fn due(warden: &Warden) -> bool {
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
        Err(e) => {
            log!("update to {version} failed: {e}");
            false
        }
    }
}

pub fn after_run(result: Result<(), String>, updated: bool) -> Result<(), String> {
    match (result, RESTART.get()) {
        (Ok(()), Some(exe)) => relaunch(exe, Some(VERSION)),

        (Err(e), _) if updated => {
            log!("{VERSION} failed to start, rolling back: {e}");
            rollback().map_or_else(|r| Err(format!("{e}; rollback: {r}")), |exe| relaunch(&exe, None))
        }

        (result, _) => result,
    }
}

fn managed(exe: &Path) -> bool {
    #[cfg(windows)]
    return exe.components().any(|c| c.as_os_str().eq_ignore_ascii_case("scoop"));

    #[cfg(not(windows))]
    return exe != Path::new("/usr/local/bin/astral-warden");
}

fn sibling(exe: &Path, tag: &str) -> PathBuf {
    match exe.extension() {
        Some(ext) => exe.with_extension(format!("{tag}.{}", ext.to_string_lossy())),
        None => exe.with_extension(tag),
    }
}

fn skip_file(exe: &Path) -> PathBuf {
    exe.with_file_name("astral-warden.skip")
}

fn work_dir() -> PathBuf {
    #[cfg(windows)]
    return std::env::temp_dir().join("astral-warden-update");

    #[cfg(not(windows))]
    return PathBuf::from("/var/lib/astral-warden/update");
}

fn tool(program: &str, args: &[&str]) -> Result<String, String> {
    #[cfg(windows)]
    let program = &std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("System32")
        .join(format!("{program}.exe"));

    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null());

    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);

    let out = cmd
        .output()
        .map_err(|e| format!("{}: {e}", Path::new(program).display()))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();

    match out.status.success() {
        true => Ok(text),
        false => Err(format!(
            "{}: {}",
            Path::new(program).display(),
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

    let name = format!("astral-warden-{tag}-{FLAVOR}");
    let archive = dir.join(format!("{name}.{ARCHIVE}"));
    let sums = dir.join("SHA256SUMS");
    download(tag, &format!("{name}.{ARCHIVE}"), &archive)?;
    download(tag, "SHA256SUMS", &sums)?;

    let expected = fs::read_to_string(&sums)
        .map_err(|e| format!("SHA256SUMS: {e}"))?
        .lines()
        .find_map(|line| line.strip_suffix(&format!("  {name}.{ARCHIVE}")).map(str::to_owned))
        .ok_or("archive missing from SHA256SUMS")?;

    let bytes = fs::read(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    if sha256::hex(&bytes) != expected.to_ascii_lowercase() {
        return Err("checksum mismatch".into());
    }

    let member = format!("{name}/{MEMBER}");
    let (archive, dir_arg) = (archive.to_string_lossy(), dir.to_string_lossy());
    tool("tar", &["-xf", &archive, "-C", &dir_arg, &member])?;

    let binary = dir.join(&name).join(MEMBER);
    let version = tool(&binary.to_string_lossy(), &["--version"])?;
    if version != format!("astral-warden {}", tag.trim_start_matches('v')) {
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
