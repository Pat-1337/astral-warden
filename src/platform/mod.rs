#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(target_os = "linux", windows)))]
mod other;
#[cfg(not(any(target_os = "linux", windows)))]
pub use other::*;

#[cfg(unix)]
mod unix;
