use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::PlatformError;

pub(crate) fn replace_file(from: &Path, to: &Path) -> Result<(), PlatformError> {
    std::fs::rename(from, to).map_err(|e| PlatformError::io(to, e))
}

/// Persists the rename itself; without this a crash can lose the new
/// directory entry even though the file data was synced.
pub(crate) fn sync_dir(dir: &Path) -> Result<(), PlatformError> {
    std::fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| PlatformError::io(dir, e))
}

pub(crate) fn restrict_dir(path: &Path) -> Result<(), PlatformError> {
    set_mode(path, 0o700)
}

pub(crate) fn restrict_file(path: &Path) -> Result<(), PlatformError> {
    set_mode(path, 0o600)
}

fn set_mode(path: &Path, mode: u32) -> Result<(), PlatformError> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| PlatformError::io(path, e))
}
