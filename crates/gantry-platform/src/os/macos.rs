//! macOS: compiles from M0, ported at M9 (plan §15).

use std::sync::Arc;

use keyring_core::CredentialStore;

use super::unix;
use crate::{BrowserInfo, PlatformError, ScheduleSpec, ScheduleStatus};

pub(crate) use unix::{replace_file, restrict_dir, restrict_file, sync_dir};

pub(crate) const NAME: &str = "macOS";

pub(crate) fn credential_store() -> Result<Arc<CredentialStore>, PlatformError> {
    let store: Arc<CredentialStore> = apple_native_keyring_store::keychain::Store::new()?;
    Ok(store)
}

pub(crate) fn locate_browser() -> Result<Option<BrowserInfo>, PlatformError> {
    Err(PlatformError::Unsupported("browser lookup"))
}

pub(crate) fn schedule_install(_spec: &ScheduleSpec) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported("launchd integration"))
}

pub(crate) fn schedule_remove() -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported("launchd integration"))
}

pub(crate) fn schedule_status() -> Result<ScheduleStatus, PlatformError> {
    Err(PlatformError::Unsupported("launchd integration"))
}
