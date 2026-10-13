use super::wide;
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW,
    RegEnumValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};

pub enum Value<'a> {
    Text(&'a str),
    Number(u32),
}

#[derive(Clone, Copy)]
pub enum Root {
    Machine,
    User,
}

impl Root {
    fn hkey(self) -> HKEY {
        match self {
            Self::Machine => HKEY_LOCAL_MACHINE,
            Self::User => HKEY_CURRENT_USER,
        }
    }
}

pub struct Key(HKEY);

impl Key {
    pub fn create(path: &str) -> Result<Self, String> {
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

    pub fn open(root: Root, path: &str, access: REG_SAM_FLAGS) -> Option<Self> {
        let mut key = std::ptr::null_mut();
        let path_w = wide(path);

        let status = unsafe { RegOpenKeyExW(root.hkey(), path_w.as_ptr(), 0, access, &mut key) };
        (status == ERROR_SUCCESS).then_some(Self(key))
    }

    pub fn subkeys(&self) -> Vec<String> {
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

    pub fn values(&self) -> Vec<String> {
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

    pub fn set(&self, name: &str, value: Value) -> Result<(), String> {
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

    pub fn delete_value(&self, name: &str) {
        let name_w = wide(name);
        unsafe { RegDeleteValueW(self.0, name_w.as_ptr()) };
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

pub fn delete(root: Root, path: &str) {
    let path_w = wide(path);
    unsafe { RegDeleteTreeW(root.hkey(), path_w.as_ptr()) };
}

pub fn text(root: Root, path: &str, name: &str) -> Option<String> {
    let (path, name) = (wide(path), wide(name));
    let mut value = [0u16; 512];
    let mut size = size_of_val(&value) as u32;

    let status = unsafe {
        RegGetValueW(
            root.hkey(),
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

pub fn flag(root: Root, path: &str, name: &str) -> bool {
    let (path, name) = (wide(path), wide(name));
    let (mut value, mut size) = (0u32, 4u32);

    let status = unsafe {
        RegGetValueW(
            root.hkey(),
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
