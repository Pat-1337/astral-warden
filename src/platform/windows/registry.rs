use super::wide;
use std::path::Path;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_DWORD,
    RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    RegSetValueExW,
};

const APP: &str = r"SOFTWARE\AstralWarden";
const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\AstralWarden";
const USB: &str = r"SYSTEM\CurrentControlSet\Enum\USB";

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

    fn open(path: &str) -> Option<Self> {
        let mut key = std::ptr::null_mut();
        let path_w = wide(path);

        let status = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path_w.as_ptr(), 0, KEY_READ, &mut key) };
        (status == ERROR_SUCCESS).then_some(Self(key))
    }

    fn subkeys(&self) -> impl Iterator<Item = String> + '_ {
        (0..).map_while(|index| {
            let mut name = [0u16; 256];
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

            (status == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&name[..len as usize]))
        })
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
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn delete(path: &str) {
    let path_w = wide(path);
    unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, path_w.as_ptr()) };
}

fn text(path: &str, name: &str) -> Option<String> {
    let (path, name) = (wide(path), wide(name));
    let mut value = [0u16; 64];
    let mut size = size_of_val(&value) as u32;

    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
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

pub fn usb_ports(device: &str) -> Vec<String> {
    let path = format!(r"{USB}\{device}");
    let Some(key) = Key::open(&path) else {
        return Vec::new();
    };

    key.subkeys()
        .filter_map(|instance| text(&format!(r"{path}\{instance}\Device Parameters"), "PortName"))
        .collect()
}

pub fn autostart() -> bool {
    let (path, name) = (wide(APP), wide("Autostart"));
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

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    Key::create(APP)?.set("Autostart", Value::Number(u32::from(enabled)))
}

pub fn register(exe: &Path) -> Result<(), String> {
    let dir = exe.parent().unwrap_or(exe).to_string_lossy();
    let exe_text = exe.to_string_lossy();
    let uninstall = format!("\"{exe_text}\" uninstall");
    let size_kib = std::fs::metadata(exe).map_or(0, |m| (m.len() / 1024) as u32);

    let key = Key::create(UNINSTALL)?;
    key.set("DisplayName", Value::Text("Astral Warden"))?;
    key.set("DisplayVersion", Value::Text(env!("CARGO_PKG_VERSION")))?;
    key.set("Publisher", Value::Text("Pat-1337"))?;
    key.set("URLInfoAbout", Value::Text("https://github.com/Pat-1337/astral-warden"))?;
    key.set("DisplayIcon", Value::Text(&exe_text))?;
    key.set("InstallLocation", Value::Text(&dir))?;
    key.set("UninstallString", Value::Text(&uninstall))?;
    key.set("QuietUninstallString", Value::Text(&uninstall))?;
    key.set("EstimatedSize", Value::Number(size_kib))?;
    key.set("NoModify", Value::Number(1))?;
    key.set("NoRepair", Value::Number(1))
}

pub fn unregister() {
    delete(UNINSTALL);
    delete(APP);
}
