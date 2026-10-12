mod hwinfo;
mod nvapi;
mod tray;

use crate::config::{self, Config};
use crate::guard::Phase;
use crate::hardware::{Device, It8915, no_device};
use crate::log;
use crate::sensor::{Reading, Sensor};
use crate::warden::{Host, Severity, Warden};
use hwinfo::HwInfo;
use nvapi::{NvApi, NvI2c};
use std::cell::{Cell, RefCell};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tray::{Tone, Tray};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, LUID, POINT,
    WPARAM,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetConsoleWindow};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Shutdown::{
    InitiateSystemShutdownExW, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_HARDWARE, SHTDN_REASON_MINOR_ENVIRONMENT,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Shell::{
    IsUserAnAdmin, NIN_SELECT, NINF_KEY, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, ChangeWindowMessageFilterEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, KillTimer, MB_ICONERROR, MB_ICONINFORMATION,
    MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MSGFLT_ALLOW, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_HIDE, SW_SHOWNORMAL, SetForegroundWindow,
    SetMenuDefaultItem, SetTimer, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_ENDSESSION, WM_NULL, WM_QUERYENDSESSION, WM_TIMER, WNDCLASSW,
};

const TASK: &str = "AstralWarden";
const WM_TRAY: u32 = WM_APP + 1;
const POLL_TIMER: usize = 1;
const WM_AUTOSTART: u32 = WM_APP + 2;
const MENU_INFO: usize = 9;
const MENU_AUTOSTART: usize = 13;
const MENU_CONFIG: usize = 10;
const MENU_LOG: usize = 11;
const MENU_EXIT: usize = 12;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static CONSOLE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

struct App {
    warden: Warden,
    tray: Tray,
    config: PathBuf,
    started: Instant,
    autostart: bool,
}

impl App {
    fn refresh(&mut self) {
        let tone = match self.warden.phase() {
            Phase::Normal => Tone::Ok,
            Phase::Warning { .. } | Phase::Recovering { .. } | Phase::Blind => Tone::Warn,
            Phase::Halted => Tone::Crit,
        };

        self.tray
            .update(tone, &format!("Astral Warden\n{}", self.warden.summary()));
    }
}

impl Host for Tray {
    fn notify(&mut self, severity: Severity, title: &str, body: &str) {
        self.balloon(severity, title, body);
    }

    fn shutdown(&mut self, reason: &str) -> Result<(), String> {
        power_off(reason)
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn os_error() -> String {
    std::io::Error::last_os_error().to_string()
}

struct Fallback {
    direct: Option<It8915<NvI2c>>,
    hwinfo: HwInfo,
}

impl Sensor for Fallback {
    fn read(&mut self) -> Result<Reading, String> {
        let direct = match self.direct.as_mut().map(Sensor::read) {
            Some(Ok(reading)) => return Ok(reading),
            Some(Err(e)) => format!("NVAPI: {e}; "),
            None => String::new(),
        };

        self.hwinfo.read().map_err(|e| format!("{direct}HWiNFO: {e}"))
    }
}

pub fn open_device() -> Result<Device, String> {
    let api = NvApi::load()?;
    let mut failures = Vec::new();

    for (gpu, pci) in api.gpus() {
        let Some(model) = pci.model() else {
            failures.push(format!("{pci} is not a supported board"));
            continue;
        };

        let direct = It8915::probe(NvI2c { api: api.clone(), gpu })
            .inspect_err(|e| log!("direct sensor read failed ({e}), falling back to HWiNFO"))
            .ok();
        let source = match direct {
            Some(_) => "NVAPI, HWiNFO as fallback",
            None => "HWiNFO",
        };

        return Ok(Device {
            name: format!("{model} via {source}"),
            pci: Some(pci),
            bus: api.bus(gpu),
            sensor: Box::new(Fallback {
                direct,
                hwinfo: HwInfo::new(),
            }),
        });
    }

    Err(no_device(&failures))
}

pub struct Lock(HANDLE);

impl Drop for Lock {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub fn single_instance() -> Result<Lock, String> {
    let name = wide(r"Global\AstralWarden");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    let running = || Err("Astral Warden is already running".into());

    match (handle.is_null(), unsafe { GetLastError() }) {
        (false, ERROR_ALREADY_EXISTS) => {
            unsafe { CloseHandle(handle) };
            running()
        }
        (true, ERROR_ACCESS_DENIED) => running(),
        (true, _) => Err(format!("CreateMutex: {}", os_error())),
        (false, _) => Ok(Lock(handle)),
    }
}

fn enable_shutdown_privilege() {
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
            return;
        }

        let mut luid = LUID {
            LowPart: 0,
            HighPart: 0,
        };
        let name = wide("SeShutdownPrivilege");
        if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) != 0 {
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            AdjustTokenPrivileges(token, 0, &privileges, 0, std::ptr::null_mut(), std::ptr::null_mut());
        }

        CloseHandle(token);
    }
}

fn power_off(reason: &str) -> Result<(), String> {
    enable_shutdown_privilege();

    let message = wide(reason);
    let reason_code = SHTDN_REASON_MAJOR_HARDWARE | SHTDN_REASON_MINOR_ENVIRONMENT | SHTDN_REASON_FLAG_PLANNED;
    if unsafe { InitiateSystemShutdownExW(std::ptr::null(), message.as_ptr(), 0, 1, 0, reason_code) } != 0 {
        return Ok(());
    }

    let api_error = os_error();
    hidden("shutdown", &["/s", "/f", "/t", "0"])
        .map(drop)
        .map_err(|e| format!("{api_error}; {e}"))
}

fn hidden(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;

    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned() + String::from_utf8_lossy(&out.stderr).trim();

    match out.status.success() {
        true => Ok(text),
        false => Err(format!("{program}: {text}")),
    }
}

fn log_path() -> PathBuf {
    config::default_path().with_file_name("astral-warden.log")
}

fn open_in_shell(path: &Path) {
    let (verb, file) = (wide("open"), wide(&path.to_string_lossy()));
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
}

fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

struct MenuInfo {
    title: String,
    device: String,
    rows: Vec<(String, String)>,
    autostart: bool,
    config: PathBuf,
}

fn menu_info() -> Option<MenuInfo> {
    with_app(|app| {
        let mut rows = app.warden.details();
        if let Some(mb) = memory_mb() {
            rows.push(("Memory".into(), format!("{mb:.1} MB")));
        }
        rows.push(("Uptime".into(), uptime(app.started)));

        MenuInfo {
            title: format!("Astral Warden {}", env!("CARGO_PKG_VERSION")),
            device: app.warden.name().to_owned(),
            rows,
            autostart: app.autostart,
            config: app.config.clone(),
        }
    })
}

fn show_menu(hwnd: HWND) {
    let Some(info) = menu_info() else {
        return;
    };

    let command = unsafe {
        let menu = CreatePopupMenu();
        let item = |flags: u32, id: usize, text: &str| AppendMenuW(menu, flags, id, wide(text).as_ptr());

        item(MF_STRING, MENU_INFO, &info.title);
        SetMenuDefaultItem(menu, 0, 1);
        item(MF_STRING | MF_GRAYED, MENU_INFO, &info.device);
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

        for (label, value) in &info.rows {
            item(MF_STRING, MENU_INFO, &format!("{label}\t{value}"));
        }

        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        let check = if info.autostart { MF_CHECKED } else { MF_UNCHECKED };
        item(MF_STRING | check, MENU_AUTOSTART, "Start at logon");
        item(MF_STRING, MENU_CONFIG, "Open config");
        item(MF_STRING, MENU_LOG, "Open log");
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        item(MF_STRING, MENU_EXIT, "Exit");

        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        SetForegroundWindow(hwnd);
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY;
        let command = TrackPopupMenu(menu, flags, cursor.x, cursor.y, 0, hwnd, std::ptr::null());

        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);

        command as usize
    };

    match command {
        MENU_AUTOSTART => toggle_autostart(hwnd, !info.autostart, info.config),
        MENU_CONFIG => {
            let _ = Config::write_default(&info.config);
            open_in_shell(&info.config);
        }
        MENU_LOG => open_in_shell(&log_path()),
        MENU_EXIT => unsafe {
            DestroyWindow(hwnd);
        },
        _ => {}
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam == POLL_TIMER => {
            with_app(|app| {
                app.warden.tick(&mut app.tray);
                app.refresh();
            });
            0
        }

        WM_TRAY => {
            if matches!(lparam as u32 & 0xFFFF, WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT) {
                show_menu(hwnd);
            }
            0
        }

        WM_AUTOSTART => {
            with_app(|app| {
                app.autostart = autostart_enabled();
                let (severity, body) = match (wparam != 0, lparam != 0) {
                    (true, true) => (Severity::Info, "Starts at every logon."),
                    (true, false) => (Severity::Info, "No longer starts at logon."),
                    (false, _) => (Severity::Warn, "Changing autostart failed. See the log."),
                };
                app.tray.balloon(severity, "Astral Warden", body);
            });
            0
        }

        WM_QUERYENDSESSION => 1,

        WM_ENDSESSION => {
            if wparam != 0 {
                APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut app| app.take()));
            }
            0
        }

        WM_DESTROY => {
            unsafe {
                KillTimer(hwnd, POLL_TIMER);
                PostQuitMessage(0);
            }
            0
        }

        _ if msg != 0 && msg == TASKBAR_CREATED.get() => {
            with_app(|app| app.tray.readd());
            0
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub fn run(warden: Warden, cfg: &Config, config: &Path) -> Result<(), String> {
    log::to_file(log_path());
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let class = wide("AstralWarden");

    let hwnd = unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(window_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();

        if RegisterClassW(&wc) == 0 {
            return Err(format!("RegisterClass: {}", os_error()));
        }

        TASKBAR_CREATED.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));

        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };

    if hwnd.is_null() {
        return Err(format!("CreateWindow: {}", os_error()));
    }

    unsafe { ChangeWindowMessageFilterEx(hwnd, TASKBAR_CREATED.get(), MSGFLT_ALLOW, std::ptr::null_mut()) };

    let mut app = App {
        warden,
        tray: Tray::new(hwnd, WM_TRAY),
        config: config.to_path_buf(),
        started: Instant::now(),
        autostart: autostart_enabled(),
    };

    app.refresh();
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    unsafe {
        SetTimer(hwnd, POLL_TIMER, cfg.poll().as_millis() as u32, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    APP.with(|cell| cell.borrow_mut().take());
    Ok(())
}

fn copy_with_retry(from: &Path, to: &Path) -> Result<(), String> {
    let mut result = std::fs::copy(from, to);

    for _ in 0..20 {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        result = std::fs::copy(from, to);
    }

    result.map(drop).map_err(|e| format!("{}: {e}", to.display()))
}

fn install_dir() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from)
        .join("AstralWarden")
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn task_xml(exe: &Path, config: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>12V-2x6 per-pin watchdog</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>
  <Principals>
    <Principal id="Author"><GroupId>S-1-5-32-545</GroupId><RunLevel>HighestAvailable</RunLevel></Principal>
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
    <Exec><Command>"{}"</Command><Arguments>run --wait --config "{}"</Arguments></Exec>
  </Actions>
</Task>
"#,
        xml_escape(&exe.to_string_lossy()),
        xml_escape(&config.to_string_lossy())
    )
}

fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

fn relaunch_elevated(args: &str, wait: bool) -> Result<u32, String> {
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
        let (text, title) = (wide(message), wide("Astral Warden"));
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONINFORMATION) };
    }
}

fn install_args(config: &Path, background: bool) -> String {
    let flag = if background { " --background" } else { "" };
    format!("install --config \"{}\"{flag}", config.display())
}

fn uninstall_args(background: bool) -> String {
    format!("uninstall{}", if background { " --background" } else { "" })
}

fn autostart_enabled() -> bool {
    hidden("schtasks", &["/Query", "/TN", TASK]).is_ok()
}

pub fn install(config: &Path, background: bool) -> Result<(), String> {
    if !is_admin() {
        relaunch_elevated(&install_args(config, background), false)?;
        println!("continuing elevated");
        return Ok(());
    }

    if background {
        log::to_file(log_path());
    }

    let dir = install_dir();
    let exe = dir.join("astral-warden.exe");

    if !background {
        let _ = hidden("schtasks", &["/End", "/TN", TASK]);
    }

    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if current != exe {
        copy_with_retry(&current, &exe)?;
    }

    if Config::write_default(config).map_err(|e| format!("{}: {e}", config.display()))? {
        println!("wrote {}", config.display());
    }

    let xml_path = std::env::temp_dir().join("astral-warden-task.xml");
    let bytes: Vec<u8> = [0xFEFF]
        .into_iter()
        .chain(task_xml(&exe, config).encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(&xml_path, bytes).map_err(|e| format!("{}: {e}", xml_path.display()))?;

    let created = hidden(
        "schtasks",
        &["/Create", "/F", "/TN", TASK, "/XML", &xml_path.to_string_lossy()],
    );
    let _ = std::fs::remove_file(&xml_path);
    created?;

    if background {
        log!("logon task registered for {}", exe.display());
        return Ok(());
    }

    hidden("schtasks", &["/Run", "/TN", TASK])?;
    report(&format!(
        "Installed to {} and started. It runs at every logon.",
        exe.display()
    ));
    Ok(())
}

pub fn uninstall(background: bool) -> Result<(), String> {
    if !is_admin() {
        relaunch_elevated(&uninstall_args(background), false)?;
        println!("continuing elevated");
        return Ok(());
    }

    if background {
        log::to_file(log_path());
    } else {
        let _ = hidden("schtasks", &["/End", "/TN", TASK]);
    }

    hidden("schtasks", &["/Delete", "/F", "/TN", TASK])?;

    let dir = install_dir();
    let exe = dir.join("astral-warden.exe");
    if std::env::current_exe().is_ok_and(|current| current != exe) {
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::remove_dir(&dir);
    }

    if background {
        log!("logon task removed");
        return Ok(());
    }

    report(&format!(
        "Uninstalled. Config kept at {}.",
        config::default_path().display()
    ));
    Ok(())
}

fn toggle_autostart(hwnd: HWND, enable: bool, config: PathBuf) {
    let hwnd = hwnd as isize;

    std::thread::spawn(move || {
        let ok = match (is_admin(), enable) {
            (true, true) => install(&config, true)
                .inspect_err(|e| log!("enable autostart: {e}"))
                .is_ok(),
            (true, false) => uninstall(true).inspect_err(|e| log!("disable autostart: {e}")).is_ok(),
            (false, true) => relaunch_elevated(&install_args(&config, true), true).is_ok_and(|code| code == 0),
            (false, false) => relaunch_elevated(&uninstall_args(true), true).is_ok_and(|code| code == 0),
        };

        unsafe { PostMessageW(hwnd as HWND, WM_AUTOSTART, usize::from(ok), isize::from(enable)) };
    });
}

fn memory_mb() -> Option<f64> {
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) } != 0;
    ok.then(|| counters.WorkingSetSize as f64 / (1024.0 * 1024.0))
}

fn uptime(since: Instant) -> String {
    let secs = since.elapsed().as_secs();
    match (secs / 86_400, secs / 3600 % 24, secs / 60 % 60) {
        (0, 0, m) => format!("{m} min"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

pub fn attach_console() {
    let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 || !GetConsoleWindow().is_null() };
    CONSOLE.store(attached, Ordering::Relaxed);
}

pub fn fatal(message: &str) {
    if !CONSOLE.load(Ordering::Relaxed) {
        let (text, title) = (wide(message), wide("Astral Warden"));
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR) };
    }
}
