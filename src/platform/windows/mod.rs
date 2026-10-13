mod install;
mod registry;
mod tray;

use gpu_warden::win::{os_error, wide};

pub use install::{adopt, install, uninstall};

use gpu_warden::config::{self, Config};
use gpu_warden::display;
use gpu_warden::guard::Phase;
use gpu_warden::log;
use gpu_warden::warden::{Host, Severity, Warden};
use install::{autostart_enabled, install_args, is_admin, refresh_registration, relaunch_elevated, uninstall_args};
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
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetConsoleWindow};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows_sys::Win32::System::Shutdown::{
    InitiateSystemShutdownExW, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_HARDWARE, SHTDN_REASON_MINOR_ENVIRONMENT,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Shell::{NIN_SELECT, NINF_KEY, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, ChangeWindowMessageFilterEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, KillTimer, MB_ICONERROR, MB_ICONHAND, MB_ICONWARNING,
    MB_SETFOREGROUND, MB_TOPMOST, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MSGFLT_ALLOW,
    MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_SHOWNORMAL,
    SetForegroundWindow, SetMenuDefaultItem, SetTimer, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_ENDSESSION, WM_NULL, WM_QUERYENDSESSION, WM_TIMER,
    WNDCLASSW,
};

const WINDOW_CLASS: &str = "GPUWarden";
const WM_TRAY: u32 = WM_APP + 1;
const POLL_TIMER: usize = 1;
const WM_AUTOSTART: u32 = WM_APP + 2;
const WM_AUTO_UPDATE: u32 = WM_APP + 3;
const TIP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const MENU_INFO: usize = 9;
const MENU_AUTOSTART: usize = 13;
const MENU_CONFIG: usize = 10;
const MENU_LOG: usize = 11;
const MENU_EXIT: usize = 12;
const MENU_UPDATE: usize = 14;
const MENU_AUTO_UPDATE: usize = 15;
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
    auto_update: bool,
    shown: Option<(Tone, Instant)>,
    greet: bool,
}

impl App {
    fn refresh(&mut self) {
        let tone = match self.warden.phase() {
            Phase::Normal => Tone::Ok,
            Phase::Warning { .. } | Phase::Recovering { .. } | Phase::Blind => Tone::Warn,
            Phase::Halted => Tone::Crit,
        };

        if self
            .shown
            .is_some_and(|(shown, at)| shown == tone && at.elapsed() < TIP_INTERVAL)
        {
            return;
        }

        self.shown = Some((tone, Instant::now()));
        self.tray
            .update(tone, &format!("{}\n{}", display::app(), self.warden.summary()));
    }
}

impl Host for Tray {
    fn notify(&mut self, severity: Severity, title: &str, body: &str) {
        self.balloon(severity, title, body);
    }

    fn alert(&mut self, title: &str, body: &str) {
        let (title, body) = (wide(title), wide(body));
        std::thread::spawn(move || unsafe {
            let flags = MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND;
            MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), flags);
        });
    }

    fn alarm(&mut self, severity: Severity) {
        let alias = wide(match severity {
            Severity::Crit => "SystemHand",
            Severity::Info | Severity::Warn => "SystemExclamation",
        });

        let played = unsafe {
            PlaySoundW(
                alias.as_ptr(),
                std::ptr::null_mut(),
                SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
            )
        } != 0;
        if !played {
            unsafe { MessageBeep(MB_ICONHAND) };
        }
    }

    fn shutdown(&mut self, reason: &str) -> Result<(), String> {
        power_off(reason)
    }
}

pub struct Lock(HANDLE);

impl Drop for Lock {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub fn single_instance() -> Result<Lock, String> {
    let name = wide(r"Global\GPUWarden");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    let running = || Err("GPU Warden is already running".into());

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
    #[cfg(feature = "portable")]
    return config::default_path().with_extension("log");

    #[cfg(not(feature = "portable"))]
    return config::default_path().with_file_name("gpu-warden.log");
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
    auto_update: bool,
    config: PathBuf,
}

fn menu_info() -> Option<MenuInfo> {
    with_app(|app| {
        let mut rows = Vec::new();
        app.warden
            .details(|key, value| rows.push((key.to_owned(), value.to_string())));
        if let Some((working, private)) = memory() {
            let memory = match display::hex() {
                true => format!(
                    "{} ({} private)",
                    display::count((working * 1024.0) as u64, "KiB"),
                    display::count((private * 1024.0) as u64, "KiB")
                ),
                false => format!("{working:.1} MB ({private:.1} private)"),
            };
            rows.push(("Memory".into(), memory));
        }
        rows.push(("Uptime".into(), uptime(app.started)));

        MenuInfo {
            title: format!("{} {}", display::app(), env!("CARGO_PKG_VERSION")),
            device: app.warden.name().to_owned(),
            rows,
            autostart: app.autostart,
            auto_update: app.auto_update,
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
        let check = match (crate::update::available(), info.auto_update) {
            (false, _) => MF_GRAYED | MF_UNCHECKED,
            (true, true) => MF_CHECKED,
            (true, false) => MF_UNCHECKED,
        };
        item(MF_STRING | check, MENU_AUTO_UPDATE, "Auto update");
        item(MF_STRING, MENU_UPDATE, "Check for updates");
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
        MENU_UPDATE => crate::update::check_now(),
        MENU_AUTO_UPDATE => toggle_auto_update(hwnd, !info.auto_update, info.config),
        MENU_CONFIG => {
            let _ = Config::write_default(&info.config);
            open_in_shell(&info.config);
        }
        MENU_LOG if log::has_file() => open_in_shell(&log_path()),
        MENU_LOG => {
            with_app(|app| {
                app.tray.balloon(
                    Severity::Info,
                    display::app(),
                    "Nothing logged yet. The log starts at the first warning.",
                )
            });
        }
        MENU_EXIT => unsafe {
            DestroyWindow(hwnd);
        },
        _ => {}
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam == POLL_TIMER => {
            let restart = with_app(|app| {
                app.warden.tick(&mut app.tray);
                app.refresh();

                if std::mem::take(&mut app.greet) {
                    let (severity, title, body) = app.warden.greeting();
                    app.tray.balloon(severity, &title, &body);
                }

                crate::update::tick(&app.warden, &mut app.tray)
            });

            if restart == Some(true) {
                unsafe { DestroyWindow(hwnd) };
            }
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
                app.tray.balloon(severity, display::app(), body);
            });
            0
        }

        WM_AUTO_UPDATE => {
            with_app(|app| {
                let (ok, enabled) = (wparam != 0, lparam != 0);
                if ok {
                    app.auto_update = enabled;
                }

                let (severity, body) = match (ok, enabled) {
                    (true, true) => (Severity::Info, "Auto update on. New versions install at startup."),
                    (true, false) => (Severity::Info, "Auto update off."),
                    (false, _) => (Severity::Warn, "Changing auto update failed. See the log."),
                };
                app.tray.balloon(severity, display::app(), body);
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

pub fn run(warden: Warden, cfg: &Config, config: &Path, silent: bool) -> Result<(), String> {
    log::to_file(log_path());
    refresh_registration();
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let class = wide(WINDOW_CLASS);

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
        auto_update: cfg.auto_update,
        shown: None,
        greet: !silent,
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

fn failed(what: &str, error: &str) {
    log::arm();
    log!("{what}: {error}");
}

fn toggle_autostart(hwnd: HWND, enable: bool, config: PathBuf) {
    let hwnd = hwnd as isize;

    std::thread::spawn(move || {
        let ok = match (is_admin(), enable) {
            (true, true) => install(&config, true)
                .inspect_err(|e| failed("enable autostart", e))
                .is_ok(),
            (true, false) => uninstall(true).inspect_err(|e| failed("disable autostart", e)).is_ok(),
            (false, true) => relaunch_elevated(&install_args(&config, true), true).is_ok_and(|code| code == 0),
            (false, false) => relaunch_elevated(&uninstall_args(true), true).is_ok_and(|code| code == 0),
        };

        unsafe { PostMessageW(hwnd as HWND, WM_AUTOSTART, usize::from(ok), isize::from(enable)) };
    });
}

fn toggle_auto_update(hwnd: HWND, enable: bool, config: PathBuf) {
    let hwnd = hwnd as isize;

    std::thread::spawn(move || {
        let ok = match is_admin() {
            true => Config::set_auto_update(&config, enable)
                .inspect_err(|e| failed("change auto update", e))
                .is_ok(),
            false => {
                let args = format!(
                    "auto-update {} --config \"{}\"",
                    if enable { "on" } else { "off" },
                    config.display()
                );
                relaunch_elevated(&args, true).is_ok_and(|code| code == 0)
            }
        };

        unsafe { PostMessageW(hwnd as HWND, WM_AUTO_UPDATE, usize::from(ok), isize::from(enable)) };
    });
}

pub fn memory() -> Option<(f64, f64)> {
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    let size = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), size) } != 0;

    let mb = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    ok.then(|| (mb(counters.WorkingSetSize), mb(counters.PrivateUsage)))
}

fn uptime(since: Instant) -> String {
    let secs = since.elapsed().as_secs();
    if display::hex() {
        return display::count(secs, "s").to_string();
    }

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
        let (text, title) = (
            wide(&format!("GPU Warden is not running.\n\n{message}")),
            wide("GPU Warden"),
        );
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR) };
    }
}
