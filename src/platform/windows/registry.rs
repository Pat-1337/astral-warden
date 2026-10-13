use super::wide;
use std::path::Path;
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};

const APP: &str = r"SOFTWARE\GPUWarden";
const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\GPUWarden";
const LEGACY_APP: &str = r"SOFTWARE\AstralWarden";
const LEGACY_UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\AstralWarden";
const USB: &str = r"SYSTEM\CurrentControlSet\Enum\USB";

const MUI_CACHE: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache";
const COMPATIBILITY: &str =
    r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Compatibility Assistant\Store";
const NOTIFY_ICONS: &str = r"Control Panel\NotifyIconSettings";
const USER_ASSIST: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist";

enum Value<'a> {
    Text(&'a str),
    Number(u32),
}

struct Key(HKEY);

impl Key {
    fn create(path: &str) -> Result<Self, String> {
        let mut key = std::ptr::null_mut();
        let path_w = wide(path);

        let status = unsafe {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                path_w.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };

        match status {
            ERROR_SUCCESS => Ok(Self(key)),
            code => Err(format!(r"HKLM\{path}: error {code}")),
        }
    }

    fn open(root: HKEY, path: &str, access: REG_SAM_FLAGS) -> Option<Self> {
        let mut key = std::ptr::null_mut();
        let path_w = wide(path);

        let status = unsafe { RegOpenKeyExW(root, path_w.as_ptr(), 0, access, &mut key) };
        (status == ERROR_SUCCESS).then_some(Self(key))
    }

    fn subkeys(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut name = [0u16; 256];

        for index in 0.. {
            let mut len = name.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };

            match status {
                ERROR_SUCCESS => names.push(String::from_utf16_lossy(&name[..len as usize])),
                ERROR_MORE_DATA => continue,
                _ => break,
            }
        }

        names
    }

    fn values(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut name = vec![0u16; 2048];

        for index in 0.. {
            let mut len = name.len() as u32;
            let status = unsafe {
                RegEnumValueW(
                    self.0,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };

            match status {
                ERROR_SUCCESS => names.push(String::from_utf16_lossy(&name[..len as usize])),
                ERROR_MORE_DATA => continue,
                _ => break,
            }
        }

        names
    }

    fn set(&self, name: &str, value: Value) -> Result<(), String> {
        let name_w = wide(name);
        let (kind, bytes) = match value {
            Value::Text(text) => (REG_SZ, wide(text).iter().flat_map(|c| c.to_le_bytes()).collect()),
            Value::Number(n) => (REG_DWORD, n.to_le_bytes().to_vec()),
        };

        match unsafe { RegSetValueExW(self.0, name_w.as_ptr(), 0, kind, bytes.as_ptr(), bytes.len() as u32) } {
            ERROR_SUCCESS => Ok(()),
            code => Err(format!("{name}: error {code}")),
        }
    }

    fn delete_value(&self, name: &str) {
        let name_w = wide(name);
        unsafe { RegDeleteValueW(self.0, name_w.as_ptr()) };
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn delete(root: HKEY, path: &str) {
    let path_w = wide(path);
    unsafe { RegDeleteTreeW(root, path_w.as_ptr()) };
}

fn text(root: HKEY, path: &str, name: &str) -> Option<String> {
    let (path, name) = (wide(path), wide(name));
    let mut value = [0u16; 512];
    let mut size = size_of_val(&value) as u32;

    let status = unsafe {
        RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut size,
        )
    };

    let len = (size as usize / 2).saturating_sub(1);
    (status == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&value[..len]))
}

fn flag(path: &str, name: &str) -> bool {
    let (path, name) = (wide(path), wide(name));
    let (mut value, mut size) = (0u32, 4u32);

    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };

    status == ERROR_SUCCESS && value == 1
}

pub fn usb_ports(device: &str) -> Vec<String> {
    let path = format!(r"{USB}\{device}");
    let Some(key) = Key::open(HKEY_LOCAL_MACHINE, &path, KEY_READ) else {
        return Vec::new();
    };

    key.subkeys()
        .iter()
        .filter_map(|instance| {
            text(
                HKEY_LOCAL_MACHINE,
                &format!(r"{path}\{instance}\Device Parameters"),
                "PortName",
            )
        })
        .collect()
}

pub fn autostart() -> bool {
    flag(APP, "Autostart")
}

pub fn legacy_autostart() -> bool {
    flag(LEGACY_APP, "Autostart")
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    Key::create(APP)?.set("Autostart", Value::Number(u32::from(enabled)))
}

pub fn register(exe: &Path) -> Result<(), String> {
    let dir = exe.parent().unwrap_or(exe).to_string_lossy();
    let exe_text = exe.to_string_lossy();
    let uninstall = format!("\"{exe_text}\" uninstall");
    let size_kib = std::fs::metadata(exe).map_or(0, |m| (m.len() / 1024) as u32);

    let key = Key::create(UNINSTALL)?;
    key.set("DisplayName", Value::Text("GPU Warden"))?;
    key.set("DisplayVersion", Value::Text(env!("CARGO_PKG_VERSION")))?;
    key.set("Publisher", Value::Text("Pat-1337"))?;
    key.set("URLInfoAbout", Value::Text("https://github.com/Pat-1337/gpu-warden"))?;
    key.set("DisplayIcon", Value::Text(&exe_text))?;
    key.set("InstallLocation", Value::Text(&dir))?;
    key.set("UninstallString", Value::Text(&uninstall))?;
    key.set("QuietUninstallString", Value::Text(&uninstall))?;
    key.set("EstimatedSize", Value::Number(size_kib))?;
    key.set("NoModify", Value::Number(1))?;
    key.set("NoRepair", Value::Number(1))
}

pub fn unregister() {
    delete(HKEY_LOCAL_MACHINE, UNINSTALL);
    delete(HKEY_LOCAL_MACHINE, APP);
}

pub fn unregister_legacy() {
    delete(HKEY_LOCAL_MACHINE, LEGACY_UNINSTALL);
    delete(HKEY_LOCAL_MACHINE, LEGACY_APP);
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
    let Some(key) = Key::open(HKEY_CURRENT_USER, path, KEY_READ | KEY_SET_VALUE) else {
        return;
    };

    for name in key.values().iter().filter(|name| ours(&decode(name))) {
        key.delete_value(name);
    }
}

pub fn forget_user_traces(ours: impl Fn(&str) -> bool) {
    forget_values(MUI_CACHE, &ours, str::to_owned);
    forget_values(COMPATIBILITY, &ours, str::to_owned);

    if let Some(key) = Key::open(HKEY_CURRENT_USER, USER_ASSIST, KEY_READ) {
        for guid in key.subkeys() {
            forget_values(&format!(r"{USER_ASSIST}\{guid}\Count"), &ours, rot13);
        }
    }

    if let Some(key) = Key::open(HKEY_CURRENT_USER, NOTIFY_ICONS, KEY_READ) {
        for icon in key.subkeys() {
            let path = format!(r"{NOTIFY_ICONS}\{icon}");
            if text(HKEY_CURRENT_USER, &path, "ExecutablePath").is_some_and(|exe| ours(&exe)) {
                delete(HKEY_CURRENT_USER, &path);
            }
        }
    }
}
