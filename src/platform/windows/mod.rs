mod nvapi;
mod tray;

use crate::config::{self, Config};
use crate::guard::Phase;
use crate::hardware::{Device, It8915, no_device};
use crate::log;
use crate::warden::{Host, Severity, Warden};
use nvapi::{NvApi, NvI2c};
use std::cell::{Cell, RefCell};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
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
use windows_sys::Win32::System::Shutdown::{
    InitiateSystemShutdownExW, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_HARDWARE, SHTDN_REASON_MINOR_ENVIRONMENT,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, NIN_SELECT, NINF_KEY, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, KillTimer, MB_ICONERROR, MB_ICONINFORMATION, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG,
    MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_HIDE, SW_SHOWNORMAL,
    SetForegroundWindow, SetTimer, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_ENDSESSION, WM_NULL, WM_QUERYENDSESSION, WM_TIMER, WNDCLASSW,
};

const TASK: &str = "AstralWarden";
const WM_TRAY: u32 = WM_APP + 1;
const POLL_TIMER: usize = 1;
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
    halted: bool,
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

pub fn open_device() -> Result<Device, String> {
    let api = NvApi::load()?;
    let mut failures = Vec::new();
    for (gpu, pci) in api.gpus() {
        let Some(model) = pci.model() else {
            failures.push(format!("{pci} is not a supported board"));
            continue;
        };
        match It8915::probe(NvI2c { api: api.clone(), gpu }) {
            Ok(sensor) => {
                return Ok(Device {
                    name: format!("{model} via NVAPI"),
                    pci: Some(pci),
                    sensor: Box::new(sensor),
                });
            }
            Err(e) => failures.push(format!("{model}: {e}")),
        }
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

fn show_menu(hwnd: HWND) {
    let summary = with_app(|app| app.warden.summary()).unwrap_or_default();
    let command = unsafe {
        let menu = CreatePopupMenu();
        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, wide(&summary).as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, MENU_CONFIG, wide("Open config").as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_LOG, wide("Open log").as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, MENU_EXIT, wide("Exit").as_ptr());
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
        MENU_CONFIG => {
            let path = config::default_path();
            let _ = Config::write_default(&path);
            open_in_shell(&path);
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
                if !app.halted {
                    app.halted = app.warden.tick(&mut app.tray).is_some();
                    app.refresh();
                }
            });
            0
        }
        WM_TRAY => {
            if matches!(lparam as u32 & 0xFFFF, WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT) {
                show_menu(hwnd);
            }
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

pub fn run(warden: Warden, cfg: &Config) -> Result<(), String> {
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
    let mut app = App {
        warden,
        tray: Tray::new(hwnd, WM_TRAY),
        halted: false,
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
    <Exec><Command>"{}"</Command><Arguments>run --config "{}"</Arguments></Exec>
  </Actions>
</Task>
"#,
        xml_escape(&exe.to_string_lossy()),
        xml_escape(&config.to_string_lossy())
    )
}

fn elevated(args: &str) -> Result<bool, String> {
    if unsafe { IsUserAnAdmin() } != 0 {
        return Ok(false);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (verb, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(args));
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_HIDE,
        )
    };
    match code as isize > 32 {
        true => Ok(true),
        false => Err("administrator rights are required".into()),
    }
}

fn report(message: &str) {
    println!("{message}");
    if !CONSOLE.load(Ordering::Relaxed) {
        let (text, title) = (wide(message), wide("Astral Warden"));
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONINFORMATION) };
    }
}

pub fn install(config: &Path) -> Result<(), String> {
    if elevated(&format!("install --config \"{}\"", config.display()))? {
        println!("continuing elevated");
        return Ok(());
    }
    let dir = install_dir();
    let exe = dir.join("astral-warden.exe");
    let _ = hidden("schtasks", &["/End", "/TN", TASK]);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e} (run as administrator)", dir.display()))?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if current != exe {
        std::fs::copy(&current, &exe).map_err(|e| format!("{}: {e}", exe.display()))?;
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
    created.map_err(|e| format!("{e} (run as administrator)"))?;
    hidden("schtasks", &["/Run", "/TN", TASK])?;
    report(&format!(
        "Installed to {} and started. It runs at every logon.",
        exe.display()
    ));
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    if elevated("uninstall")? {
        println!("continuing elevated");
        return Ok(());
    }
    let _ = hidden("schtasks", &["/End", "/TN", TASK]);
    hidden("schtasks", &["/Delete", "/F", "/TN", TASK]).map_err(|e| format!("{e} (run as administrator)"))?;
    let dir = install_dir();
    let exe = dir.join("astral-warden.exe");
    if std::env::current_exe().is_ok_and(|current| current != exe) {
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::remove_dir(&dir);
    }
    report(&format!(
        "Uninstalled. Config kept at {}.",
        config::default_path().display()
    ));
    Ok(())
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
