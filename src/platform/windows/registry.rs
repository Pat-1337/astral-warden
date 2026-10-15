use gpu_warden::win::registry::{Key, Root, Value, delete, flag, text};
use std::path::{Path, PathBuf};
use windows_sys::Win32::System::Registry::{KEY_READ, KEY_SET_VALUE};

const APP: &str = r"SOFTWARE\GPUWarden";
const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\GPUWarden";

const MUI_CACHE: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache";
const COMPATIBILITY: &str =
    r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Compatibility Assistant\Store";
const NOTIFY_ICONS: &str = r"Control Panel\NotifyIconSettings";
const USER_ASSIST: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist";

pub fn autostart() -> bool {
    flag(Root::Machine, APP, "Autostart")
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    Key::create(APP)?.set("Autostart", Value::Number(u32::from(enabled)))
}

pub fn install_dir() -> Option<PathBuf> {
    text(Root::Machine, APP, "InstallDir").map(PathBuf::from)
}

pub fn scope() -> Option<String> {
    text(Root::Machine, APP, "Scope")
}

#[cfg(not(feature = "portable"))]
pub fn set_install(dir: &Path, scope: &str) -> Result<(), String> {
    let key = Key::create(APP)?;
    key.set("InstallDir", Value::Text(&dir.to_string_lossy()))?;
    key.set("Scope", Value::Text(scope))
}

#[cfg(feature = "portable")]
pub fn set_scope(scope: &str) -> Result<(), String> {
    Key::create(APP)?.set("Scope", Value::Text(scope))
}

pub fn register(exe: &Path) -> Result<(), String> {
    let dir = exe.parent().unwrap_or(exe).to_string_lossy();
    let exe_text = exe.to_string_lossy();
    let uninstall = format!("\"{exe_text}\" uninstall");
    let size_kib = std::fs::metadata(exe).map_or(0, |m| (m.len() / 1024) as u32);

    let key = Key::create(UNINSTALL)?;
    key.set("DisplayName", Value::Text("GPU Warden"))?;
    key.set("DisplayVersion", Value::Text(env!("CARGO_PKG_VERSION")))?;
    key.set("Publisher", Value::Text("patvuk"))?;
    key.set("URLInfoAbout", Value::Text("https://github.com/patvuk/gpu-warden"))?;
    key.set("DisplayIcon", Value::Text(&exe_text))?;
    key.set("InstallLocation", Value::Text(&dir))?;
    key.set("UninstallString", Value::Text(&uninstall))?;
    key.set("QuietUninstallString", Value::Text(&uninstall))?;
    key.set("EstimatedSize", Value::Number(size_kib))?;
    key.set("NoModify", Value::Number(1))?;
    key.set("NoRepair", Value::Number(1))
}

pub fn unregister() {
    delete(Root::Machine, UNINSTALL);
    delete(Root::Machine, APP);
}

fn rot13(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'a'..='m' | 'A'..='M' => (c as u8 + 13) as char,
            'n'..='z' | 'N'..='Z' => (c as u8 - 13) as char,
            _ => c,
        })
        .collect()
}

fn forget_values(path: &str, ours: &impl Fn(&str) -> bool, decode: fn(&str) -> String) {
    let Some(key) = Key::open(Root::User, path, KEY_READ | KEY_SET_VALUE) else {
        return;
    };

    for name in key.values().iter().filter(|name| ours(&decode(name))) {
        key.delete_value(name);
    }
}

pub fn forget_user_traces(ours: impl Fn(&str) -> bool) {
    forget_values(MUI_CACHE, &ours, str::to_owned);
    forget_values(COMPATIBILITY, &ours, str::to_owned);

    if let Some(key) = Key::open(Root::User, USER_ASSIST, KEY_READ) {
        for guid in key.subkeys() {
            forget_values(&format!(r"{USER_ASSIST}\{guid}\Count"), &ours, rot13);
        }
    }

    if let Some(key) = Key::open(Root::User, NOTIFY_ICONS, KEY_READ) {
        for icon in key.subkeys() {
            let path = format!(r"{NOTIFY_ICONS}\{icon}");
            if text(Root::User, &path, "ExecutablePath").is_some_and(|exe| ours(&exe)) {
                delete(Root::User, &path);
            }
        }
    }
}
