//! Windows: compiles from M0, ported at M9 (plan §15).

use std::path::Path;
use std::sync::Arc;

use keyring_core::CredentialStore;

use crate::{BrowserInfo, PlatformError, ScheduleSpec, ScheduleStatus};

pub(crate) const NAME: &str = "Windows";

/// `std::fs::rename` maps to `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`. The
/// port switches this to `ReplaceFileW`, which also preserves ACLs.
pub(crate) fn replace_file(from: &Path, to: &Path) -> Result<(), PlatformError> {
    std::fs::rename(from, to).map_err(|e| PlatformError::io(to, e))
}

/// NTFS has no directory fsync; the rename is journaled.
pub(crate) fn sync_dir(_dir: &Path) -> Result<(), PlatformError> {
    Ok(())
}

/// Files under `%APPDATA%`/`%LOCALAPPDATA%` already inherit a per-user ACL.
/// The port tightens this explicitly (§12).
pub(crate) fn restrict_dir(_path: &Path) -> Result<(), PlatformError> {
    Ok(())
}

pub(crate) fn restrict_file(_path: &Path) -> Result<(), PlatformError> {
    Ok(())
}

pub(crate) fn credential_store() -> Result<Arc<CredentialStore>, PlatformError> {
    let store: Arc<CredentialStore> = windows_native_keyring_store::Store::new()?;
    Ok(store)
}

pub(crate) fn locate_browser() -> Result<Option<BrowserInfo>, PlatformError> {
    Err(PlatformError::Unsupported("browser lookup"))
}

pub(crate) fn schedule_install(_spec: &ScheduleSpec) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported("Task Scheduler integration"))
}

pub(crate) fn schedule_remove() -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported("Task Scheduler integration"))
}

pub(crate) fn schedule_status() -> Result<ScheduleStatus, PlatformError> {
    Err(PlatformError::Unsupported("Task Scheduler integration"))
}
