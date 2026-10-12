use std::ffi::c_void;

pub struct Library(*mut c_void);

unsafe impl Send for Library {}

impl Library {
    pub fn open(candidates: &[&str]) -> Option<Self> {
        candidates.iter().find_map(|name| {
            let handle = Self::load(name);
            (!handle.is_null()).then_some(Self(handle))
        })
    }

    pub unsafe fn symbol<T: Copy>(&self, name: &str) -> Option<T> {
        let mut cname = name.as_bytes().to_vec();
        cname.push(0);
        let ptr = unsafe { Self::find(self.0, &cname) };
        (!ptr.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, T>(&ptr) })
    }

    #[cfg(unix)]
    fn load(name: &str) -> *mut c_void {
        let Ok(cname) = std::ffi::CString::new(name) else {
            return std::ptr::null_mut();
        };
        unsafe { libc::dlopen(cname.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) }
    }

    #[cfg(unix)]
    unsafe fn find(handle: *mut c_void, cname: &[u8]) -> *mut c_void {
        unsafe { libc::dlsym(handle, cname.as_ptr().cast()) }
    }

    #[cfg(windows)]
    fn load(name: &str) -> *mut c_void {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide.as_ptr()) }
    }

    #[cfg(windows)]
    unsafe fn find(handle: *mut c_void, cname: &[u8]) -> *mut c_void {
        unsafe { windows_sys::Win32::System::LibraryLoader::GetProcAddress(handle, cname.as_ptr()) }
            .map_or(std::ptr::null_mut(), |f| f as *mut c_void)
    }
}
