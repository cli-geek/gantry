//! Everything that differs between Linux, Windows and macOS.
//!
//! This is the only crate in the workspace allowed to contain
//! `cfg(target_os)` (or `cfg(unix)`/`cfg(windows)`). Linux is implemented;
//! Windows and macOS compile, and the parts not yet ported return
//! [`PlatformError::Unsupported`] until the port milestone.

mod browser;
mod error;
mod fs;
mod os;
mod paths;
mod scheduler;
mod secrets;

pub use browser::{BrowserInfo, BrowserLocator, OsBrowserLocator};
pub use error::PlatformError;
pub use fs::{readable_by_others, restrict_dir, restrict_file, write_atomic};
pub use paths::Paths;
pub use scheduler::{OsScheduler, ScheduleSpec, ScheduleStatus, Scheduler};
pub use secrets::{OsSecretStore, SecretStore};

/// Name of the operating system Gantry was built for, for diagnostics.
pub const OS_NAME: &str = os::NAME;
