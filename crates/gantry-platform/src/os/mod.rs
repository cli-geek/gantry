//! Per-OS implementations. Each submodule exposes the same set of free
//! functions; the rest of the crate calls them through `crate::os::*`.

#[cfg(unix)]
mod unix;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::*;

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
compile_error!("Gantry supports Linux, Windows and macOS only");
