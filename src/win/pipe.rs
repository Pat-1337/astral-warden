use super::{os_error, wide};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GetLastError, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_WRITE_DATA, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

pub const POWER: &str = r"\\.\pipe\gpu-warden-power";
const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12019b;;;IU)";
const BUFFER: u32 = 512;
const MAX_LINE: u64 = 256;
const TAKEOVER_TRIES: u32 = 60;
const TAKEOVER_WAIT: Duration = Duration::from_millis(500);
const BUSY_WAIT: Duration = Duration::from_millis(100);

struct Descriptor(*mut core::ffi::c_void);

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0 as HLOCAL) };
    }
}

fn descriptor() -> Result<Descriptor, String> {
    let mut sd = std::ptr::null_mut();
    let sddl = wide(SDDL);

    match unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        )
    } {
        0 => Err(format!("security descriptor: {}", os_error())),
        _ => Ok(Descriptor(sd)),
    }
}

fn instance(name: &[u16], sd: &Descriptor, first: bool) -> Result<HANDLE, String> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let open_mode = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    let pipe_mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;

    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            pipe_mode,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            0,
            &attributes,
        )
    };

    match handle == INVALID_HANDLE_VALUE {
        true => Err(format!("CreateNamedPipe: {}", os_error())),
        false => Ok(handle),
    }
}

fn take_over(name: &[u16], sd: &Descriptor) -> Result<HANDLE, String> {
    let mut result = instance(name, sd, true);

    for _ in 0..TAKEOVER_TRIES {
        if result.is_ok() {
            break;
        }

        std::thread::sleep(TAKEOVER_WAIT);
        result = instance(name, sd, true);
    }

    result
}

fn answer(pipe: File, handler: &(impl Fn(&str) -> String + ?Sized)) {
    let mut line = String::new();
    let read = BufReader::new(&pipe).take(MAX_LINE).read_line(&mut line);

    if read.is_ok() {
        let mut reply = handler(line.trim());
        reply.push('\n');
        let _ = (&pipe).write_all(reply.as_bytes());
    }

    unsafe {
        FlushFileBuffers(pipe.as_raw_handle());
        DisconnectNamedPipe(pipe.as_raw_handle());
    }
}

pub fn serve(name: &str, handler: impl Fn(&str) -> String + Send + Sync + 'static) -> Result<(), String> {
    let sd = descriptor()?;
    let name = wide(name);
    let handler = Arc::new(handler);
    let mut first = true;

    loop {
        let handle = match std::mem::take(&mut first) {
            true => take_over(&name, &sd)?,
            false => instance(&name, &sd, false)?,
        };

        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        let pipe = unsafe { File::from_raw_handle(handle) };

        if connected {
            let handler = handler.clone();
            std::thread::spawn(move || answer(pipe, &*handler));
        }
    }
}

fn exchange(name: &str, line: &str) -> Result<String, String> {
    let mut busy = 0;
    let pipe = loop {
        match OpenOptions::new()
            .access_mode(GENERIC_READ | FILE_WRITE_DATA)
            .open(name)
        {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) && busy < 20 => {
                busy += 1;
                std::thread::sleep(BUSY_WAIT);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("power helper is not running".into()),
            Err(e) => return Err(format!("power helper: {e}")),
            Ok(pipe) => break pipe,
        }
    };

    (&pipe)
        .write_all(format!("{line}\n").as_bytes())
        .map_err(|e| format!("power helper: {e}"))?;

    let mut reply = String::new();
    BufReader::new(&pipe)
        .take(MAX_LINE)
        .read_line(&mut reply)
        .map_err(|e| format!("power helper: {e}"))?;

    Ok(reply.trim().to_owned())
}

pub fn request(name: &'static str, line: String, timeout: Duration) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(exchange(name, &line));
    });

    rx.recv_timeout(timeout)
        .map_err(|_| "power helper did not answer".to_owned())?
}
