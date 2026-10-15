use super::scope::Scope;
use gpu_warden::win::wide;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LocalFree, WPARAM};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;
use windows_sys::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDCBF_CANCEL_BUTTON, TDE_CONTENT, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_USE_HICON_MAIN, TDM_SET_BUTTON_ELEVATION_REQUIRED_STATE, TDM_SET_ELEMENT_TEXT, TDN_BUTTON_CLICKED, TDN_CREATED,
};
use windows_sys::Win32::UI::Shell::{
    BFFM_INITIALIZED, BFFM_SETSELECTIONW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW,
    SHGetPathFromIDListW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{LoadIconW, SendMessageW};
use windows_sys::core::{BOOL, HRESULT};

const FOLDER: &str = "GPUWarden";
const INSTALL: i32 = 100;
const BROWSE: i32 = 101;
const ALL_USERS: i32 = 200;
const ONLY_ME: i32 = 201;
const S_FALSE: HRESULT = 1;
const MAX_PATH: usize = 260;

type TaskDialogIndirect = unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> HRESULT;

pub struct Choice {
    pub dir: PathBuf,
    pub scope: Scope,
}

pub fn dedicated(dir: &Path) -> PathBuf {
    match dir.file_name().is_some_and(|name| name.eq_ignore_ascii_case(FOLDER)) {
        true => dir.to_path_buf(),
        false => dir.join(FOLDER),
    }
}

pub fn current_sid() -> Option<String> {
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }

        let mut size = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let read = GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), size, &mut size) != 0;
        CloseHandle(token);

        if !read {
            return None;
        }

        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return None;
        }

        let sid = from_wide(std::slice::from_raw_parts(
            text,
            (0..).take_while(|&i| *text.add(i) != 0).count(),
        ));
        LocalFree(text.cast());
        Some(sid.to_string_lossy().into_owned())
    }
}

fn from_wide(units: &[u16]) -> OsString {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    OsString::from_wide(&units[..end])
}

fn user_name() -> String {
    let mut buffer = [0u16; 257];
    let mut size = buffer.len() as u32;

    match unsafe { GetUserNameW(buffer.as_mut_ptr(), &mut size) } {
        0 => "this account".into(),
        _ => from_wide(&buffer).to_string_lossy().into_owned(),
    }
}

fn task_dialog() -> Option<TaskDialogIndirect> {
    let name = wide("comctl32.dll");
    let lib = unsafe { LoadLibraryW(name.as_ptr()) };
    if lib.is_null() {
        return None;
    }

    let function = unsafe { GetProcAddress(lib, c"TaskDialogIndirect".as_ptr().cast()) }?;
    Some(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, TaskDialogIndirect>(function) })
}

unsafe extern "system" fn browse_callback(hwnd: HWND, msg: u32, _: LPARAM, start: LPARAM) -> i32 {
    if msg == BFFM_INITIALIZED {
        unsafe { SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, start) };
    }
    0
}

fn pick_folder(owner: HWND, current: &Path) -> Option<PathBuf> {
    unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };

    let start = wide(&current.parent().unwrap_or(current).to_string_lossy());
    let title = wide("Choose where to install GPU Warden. It goes into a GPUWarden folder inside the one you pick.");
    let mut name = [0u16; MAX_PATH];

    let info = BROWSEINFOW {
        hwndOwner: owner,
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: name.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        lpfn: Some(browse_callback),
        lParam: start.as_ptr() as LPARAM,
        iImage: 0,
    };

    let pidl = unsafe { SHBrowseForFolderW(&info) };
    if pidl.is_null() {
        return None;
    }

    let mut path = [0u16; MAX_PATH];
    let ok = unsafe { SHGetPathFromIDListW(pidl, path.as_mut_ptr()) } != 0;
    unsafe { CoTaskMemFree(pidl.cast()) };

    ok.then(|| PathBuf::from(from_wide(&path)))
}

struct State {
    dir: PathBuf,
    content: Vec<u16>,
    elevated: bool,
}

fn content(dir: &Path) -> Vec<u16> {
    wide(&format!(
        "Install folder:\n{}\n\nSettings and logs are kept in C:\\ProgramData\\gpu-warden.",
        dir.display()
    ))
}

unsafe extern "system" fn dialog_callback(hwnd: HWND, msg: u32, wparam: WPARAM, _: LPARAM, data: isize) -> HRESULT {
    let state = unsafe { &mut *(data as *mut State) };

    match msg as i32 {
        TDN_CREATED if !state.elevated => {
            unsafe {
                SendMessageW(
                    hwnd,
                    TDM_SET_BUTTON_ELEVATION_REQUIRED_STATE as u32,
                    INSTALL as WPARAM,
                    1,
                )
            };
            0
        }

        TDN_BUTTON_CLICKED if wparam as i32 == BROWSE => {
            if let Some(dir) = pick_folder(hwnd, &state.dir) {
                state.dir = dedicated(&dir);
                state.content = content(&state.dir);
                unsafe {
                    SendMessageW(
                        hwnd,
                        TDM_SET_ELEMENT_TEXT as u32,
                        TDE_CONTENT as WPARAM,
                        state.content.as_ptr() as LPARAM,
                    )
                };
            }
            S_FALSE
        }

        _ => 0,
    }
}

pub fn ask(dir: &Path, scope: &Scope, elevated: bool) -> Result<Option<Choice>, String> {
    let sid = current_sid().ok_or("could not read the current user")?;
    let Some(show) = task_dialog() else {
        return Ok(Some(Choice {
            dir: dir.to_path_buf(),
            scope: scope.clone(),
        }));
    };

    let mut state = State {
        dir: dir.to_path_buf(),
        content: content(dir),
        elevated,
    };

    let title = wide("GPU Warden Setup");
    let heading = wide(&format!("Install GPU Warden {}", env!("CARGO_PKG_VERSION")));
    let footer = wide(
        "All users: starts at every user's logon. Standard accounts get the power cap and updates through a SYSTEM helper.\nOnly me: starts at your logon only.",
    );
    let (install, browse) = (wide("Install"), wide("Change folder..."));
    let (all_users, only_me) = (wide("All users"), wide(&format!("Only me ({})", user_name())));

    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: INSTALL,
            pszButtonText: install.as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: BROWSE,
            pszButtonText: browse.as_ptr(),
        },
    ];
    let radios = [
        TASKDIALOG_BUTTON {
            nButtonID: ALL_USERS,
            pszButtonText: all_users.as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: ONLY_ME,
            pszButtonText: only_me.as_ptr(),
        },
    ];

    let mut config: TASKDIALOGCONFIG = unsafe { std::mem::zeroed() };
    config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
    config.hInstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_USE_HICON_MAIN;
    config.dwCommonButtons = TDCBF_CANCEL_BUTTON;
    config.pszWindowTitle = title.as_ptr();
    config.Anonymous1.hMainIcon = unsafe { LoadIconW(config.hInstance, 1 as _) };
    config.pszMainInstruction = heading.as_ptr();
    config.pszContent = state.content.as_ptr();
    config.cButtons = buttons.len() as u32;
    config.pButtons = buttons.as_ptr();
    config.nDefaultButton = INSTALL;
    config.cRadioButtons = radios.len() as u32;
    config.pRadioButtons = radios.as_ptr();
    config.nDefaultRadioButton = match scope {
        Scope::User(user) if *user == sid => ONLY_ME,
        _ => ALL_USERS,
    };
    config.pszFooter = footer.as_ptr();
    config.pfCallback = Some(dialog_callback);
    config.lpCallbackData = &raw mut state as isize;

    let (mut button, mut radio) = (0, 0);
    let result = unsafe { show(&config, &mut button, &mut radio, std::ptr::null_mut()) };
    if result < 0 {
        return Err(format!("setup dialog failed ({result:#010x})"));
    }

    Ok((button == INSTALL).then_some(Choice {
        dir: state.dir,
        scope: match radio {
            ONLY_ME => Scope::User(sid),
            _ => Scope::AllUsers,
        },
    }))
}
