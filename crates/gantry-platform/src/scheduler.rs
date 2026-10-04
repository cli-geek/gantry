use std::path::PathBuf;

use crate::{PlatformError, os};

/// What the OS scheduler should run, and when (local time, daily).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub hour: u8,
    pub minute: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleStatus {
    NotInstalled,
    Installed {
        enabled: bool,
        active: bool,
        detail: String,
    },
    /// An entry exists but the scheduler itself cannot be queried.
    Unavailable(String),
}

/// systemd user timer, Task Scheduler, or launchd LaunchAgent (§3.3).
/// Missed runs fire at the next opportunity on every OS.
pub trait Scheduler: std::fmt::Debug {
    fn install(&self, spec: &ScheduleSpec) -> Result<(), PlatformError>;
    fn remove(&self) -> Result<(), PlatformError>;
    fn status(&self) -> Result<ScheduleStatus, PlatformError>;
}

#[derive(Debug, Default)]
pub struct OsScheduler;

impl Scheduler for OsScheduler {
    fn install(&self, spec: &ScheduleSpec) -> Result<(), PlatformError> {
        os::schedule_install(spec)
    }

    fn remove(&self) -> Result<(), PlatformError> {
        os::schedule_remove()
    }

    fn status(&self) -> Result<ScheduleStatus, PlatformError> {
        os::schedule_status()
    }
}
