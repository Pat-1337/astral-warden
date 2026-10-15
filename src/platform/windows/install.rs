use super::{CONSOLE, WINDOW_CLASS, hidden, log_path, registry, single_instance, wide};
use gpu_warden::config::{self, Config};
use gpu_warden::log;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{
    IsUserAnAdmin, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, MB_ICONINFORMATION, MessageBoxW, PostMessageW, SW_HIDE, WM_CLOSE,
};

const TASK: &str = "GPUWarden";
const HELPER: &str = "GPUWardenPower";
const VERSION: &str = env!("CARGO_PKG_VERSION");
#[cfg(not(feature = "portable"))]
const EXE: &str = "gpu-warden.exe";
const NAMES: &[&str] = &["gpu-warden", "gpuwarden"];
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const RETRY: Duration = Duration::from_millis(250);

fn retry<T, E>(mut attempt: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    let mut result = attempt();
    for _ in 0..20 {
        if result.is_ok() {
            break;
        }

        std::thread::sleep(RETRY);
        result = attempt();
    }

    result
}

fn scoop_current(exe: &Path) -> Option<PathBuf> {
    let parts: Vec<_> = exe.components().collect();
    let apps = parts.iter().position(|c| c.as_os_str().eq_ignore_ascii_case("apps"))?;
    let scoop = parts.get(apps.checked_sub(1)?)?;

    if !scoop.as_os_str().eq_ignore_ascii_case("scoop") || parts.len() < apps + 3 {
        return None;
    }

    let mut path: PathBuf = parts[..apps + 2].iter().collect();
    path.push("current");
    path.push(exe.file_name()?);
    Some(path)
}

fn known_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map_or_else(|| PathBuf::from(fallback), PathBuf::from)
}

fn install_dir() -> PathBuf {
    known_dir("ProgramFiles", r"C:\Program Files").join("GPUWarden")
}

fn ours(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    NAMES.iter().any(|name| text.contains(name))
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn task_xml(description: &str, trigger: &str, principal: &str, exe: &Path, arguments: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>{description}</Description></RegistrationInfo>
  <Triggers><{trigger}><Enabled>true</Enabled></{trigger}></Triggers>
  <Principals>
    <Principal id="Author">{principal}<RunLevel>HighestAvailable</RunLevel></Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
    <RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>"{}"</Command><Arguments>{}</Arguments></Exec>
  </Actions>
</Task>
"#,
        xml_escape(&exe.to_string_lossy()),
        xml_escape(arguments)
    )
}

fn logon_xml(exe: &Path, config: &Path) -> String {
    let arguments = format!("run --wait --silent --config \"{}\"", config.display());
    task_xml(
        "12V-2x6 per-pin watchdog",
        "LogonTrigger",
        "<GroupId>S-1-5-32-545</GroupId>",
        exe,
        &arguments,
    )
}

fn helper_xml(exe: &Path) -> String {
    task_xml(
        "Sets the GPU power limit for GPU Warden when it runs without administrator rights",
        "BootTrigger",
        "<UserId>S-1-5-18</UserId>",
        exe,
        "power-helper",
    )
}

fn shared(exe: &Path) -> bool {
    cfg!(not(feature = "portable")) && exe.starts_with(install_dir())
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

pub fn relaunch_elevated(args: &str, wait: bool) -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (verb, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(args));

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_HIDE;

    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err("administrator rights are required".into());
    }

    let mut code = 0;
    unsafe {
        if wait {
            WaitForSingleObject(info.hProcess, INFINITE);
            GetExitCodeProcess(info.hProcess, &mut code);
        }
        CloseHandle(info.hProcess);
    }

    Ok(code)
}

fn report(message: &str) {
    println!("{message}");
    if !CONSOLE.load(Ordering::Relaxed) {
        let (text, title) = (wide(message), wide("GPU Warden"));
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONINFORMATION) };
    }
}

pub fn install_args(config: &Path, background: bool) -> String {
    let flag = if background { " --background" } else { "" };
    format!("install --config \"{}\"{flag}", config.display())
}

pub fn uninstall_args(background: bool) -> String {
    format!("uninstall{}", if background { " --background" } else { "" })
}

pub fn autostart_enabled() -> bool {
    registry::autostart() || hidden("schtasks", &["/Query", "/TN", TASK]).is_ok()
}

#[cfg(not(feature = "portable"))]
fn target(current: &Path) -> Result<PathBuf, String> {
    if let Some(scoop) = scoop_current(current) {
        return Ok(scoop);
    }

    let dir = install_dir();
    let exe = dir.join(EXE);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    if current != exe {
        retry(|| std::fs::copy(current, &exe)).map_err(|e| format!("{}: {e}", exe.display()))?;
    }

    registry::register(&exe)?;
    Ok(exe)
}

#[cfg(feature = "portable")]
fn target(current: &Path) -> Result<PathBuf, String> {
    Ok(scoop_current(current).unwrap_or_else(|| current.to_path_buf()))
}

fn register_task(name: &str, xml: &str) -> Result<(), String> {
    let xml_path = std::env::temp_dir().join(format!("gpu-warden-{name}.xml"));
    let bytes: Vec<u8> = [0xFEFF]
        .into_iter()
        .chain(xml.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(&xml_path, bytes).map_err(|e| format!("{}: {e}", xml_path.display()))?;

    let created = hidden(
        "schtasks",
        &["/Create", "/F", "/TN", name, "/XML", &xml_path.to_string_lossy()],
    );
    let _ = std::fs::remove_file(&xml_path);
    created.map(drop)
}

fn create_task(exe: &Path, config: &Path) -> Result<(), String> {
    register_task(TASK, &logon_xml(exe, config))?;
    registry::set_autostart(true)
}

fn start_helper(exe: &Path) -> Result<(), String> {
    let _ = hidden("schtasks", &["/End", "/TN", HELPER]);
    register_task(HELPER, &helper_xml(exe))?;
    hidden("schtasks", &["/Run", "/TN", HELPER]).map(drop)
}

fn stop_helper() {
    let _ = hidden("schtasks", &["/End", "/TN", HELPER]);
    let _ = hidden("schtasks", &["/Delete", "/F", "/TN", HELPER]);
}

pub fn power_helper() -> Result<(), String> {
    CONSOLE.store(true, Ordering::Relaxed);
    log::to_file(log_path().with_file_name("power-helper.log"));
    log::arm();
    log!("power helper {VERSION} started");

    if let Ok(exe) = std::env::current_exe()
        && shared(&exe)
    {
        let _ = registry::register(&exe);
    }

    gpu_warden::power::serve(crate::update::serve)
}

pub fn install(config: &Path, background: bool) -> Result<(), String> {
    if !is_admin() {
        relaunch_elevated(&install_args(config, background), false)?;
        println!("continuing elevated");
        return Ok(());
    }

    if background {
        log::to_file(log_path());
    } else {
        let _ = hidden("schtasks", &["/End", "/TN", TASK]);
        let _ = hidden("schtasks", &["/End", "/TN", HELPER]);
    }

    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = target(&current)?;

    if Config::write_default(config).map_err(|e| format!("{}: {e}", config.display()))? {
        println!("wrote {}", config.display());
    }

    create_task(&exe, config)?;

    if shared(&exe) && gpu_warden::power::helper_version().as_deref() != Some(VERSION) {
        start_helper(&exe)?;
    }

    if background {
        log!("logon task registered for {}", exe.display());
        return Ok(());
    }

    hidden("schtasks", &["/Run", "/TN", TASK])?;

    #[cfg(not(feature = "portable"))]
    report(&format!(
        "Installed to {} and started. It runs at every logon, and the downloaded file can be deleted.",
        exe.display()
    ));

    #[cfg(feature = "portable")]
    report(&format!("Started. {} now runs at every logon.", exe.display()));

    Ok(())
}

#[cfg(not(feature = "portable"))]
pub fn adopt(config: &Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };

    if std::env::args().len() > 1 || current.starts_with(install_dir()) || scoop_current(&current).is_some() {
        return false;
    }

    if let Err(e) = install(config, false) {
        report(&format!(
            "GPU Warden was not installed: {e}.\n\nThe portable version runs without installing."
        ));
    }

    true
}

#[cfg(feature = "portable")]
pub fn adopt(_: &Path) -> bool {
    false
}

pub fn refresh_registration() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };

    if !is_admin() || !shared(&current) {
        return;
    }

    let _ = registry::register(&current);

    if gpu_warden::power::helper_version().as_deref() != Some(VERSION)
        && let Err(e) = start_helper(&current)
    {
        log!("power helper: {e}");
    }
}

fn close_windows(class: &str) {
    let class = wide(class);

    for _ in 0..50 {
        let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
        if hwnd.is_null() {
            return;
        }

        unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn close_running() {
    close_windows(WINDOW_CLASS);
    let _ = retry(single_instance);
}

fn remove_dir(dir: &Path) {
    let inside = std::env::current_exe().is_ok_and(|exe| exe.starts_with(dir));

    if inside {
        let script = format!("/C ping -n 5 127.0.0.1 >NUL & rmdir /S /Q \"{}\"", dir.display());
        let _ = Command::new("cmd")
            .raw_arg(script)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        return;
    }

    let _ = retry(|| match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    });
}

fn remove_leftovers(exe: &Path) {
    for extension in ["old.exe", "new.exe", "failed.exe", "skip"] {
        let _ = std::fs::remove_file(exe.with_extension(extension));
    }

    remove_dir(&std::env::temp_dir().join("gpu-warden-update"));
}

fn forget_traces() {
    registry::forget_user_traces(ours);

    let prefetch = known_dir("SystemRoot", r"C:\Windows").join("Prefetch");
    for file in std::fs::read_dir(prefetch).into_iter().flatten().flatten() {
        if ours(&file.file_name().to_string_lossy()) {
            let _ = std::fs::remove_file(file.path());
        }
    }
}

#[cfg(not(feature = "portable"))]
fn remove_data() {
    if let Some(dir) = config::default_path().parent() {
        remove_dir(dir);
    }
}

#[cfg(feature = "portable")]
fn remove_data() {
    let files = [
        config::default_path(),
        config::state_path(),
        config::note_path(),
        log_path(),
    ];
    for file in files.iter().chain([&log_path().with_extension("log.old")]) {
        let _ = std::fs::remove_file(file);
    }
}

pub fn uninstall(background: bool) -> Result<(), String> {
    if !is_admin() {
        relaunch_elevated(&uninstall_args(background), false)?;
        println!("continuing elevated");
        return Ok(());
    }

    if background {
        log::to_file(log_path());
        hidden("schtasks", &["/Delete", "/F", "/TN", TASK])?;
        registry::set_autostart(false)?;
        log!("logon task removed");
        return Ok(());
    }

    let _ = hidden("schtasks", &["/End", "/TN", TASK]);
    let _ = hidden("schtasks", &["/Delete", "/F", "/TN", TASK]);
    stop_helper();
    close_running();

    registry::unregister();
    remove_data();
    forget_traces();

    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    remove_leftovers(&current);

    #[cfg(not(feature = "portable"))]
    remove_dir(&install_dir());

    #[cfg(not(feature = "portable"))]
    report("GPU Warden is uninstalled. Nothing is left behind.");

    #[cfg(feature = "portable")]
    report(&format!(
        "GPU Warden is removed: logon task, settings, and logs. Delete {} to finish.",
        current.display()
    ));

    Ok(())
}
