use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::PlatformError;
use crate::fs::restrict_dir;

/// Where Gantry keeps its files.
///
/// Config holds the TOML profile and banks (§9.1); data holds the SQLite
/// database (§9.2). Both can be overridden, which is how tests and fixture
/// profiles run against isolated directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_dir: PathBuf,
    data_dir: PathBuf,
}

impl Paths {
    /// The per-user OS locations: `$XDG_CONFIG_HOME/gantry` and
    /// `$XDG_DATA_HOME/gantry` on Linux, `%APPDATA%` and `%LOCALAPPDATA%` on
    /// Windows, `~/Library/Application Support` on macOS.
    pub fn from_os() -> Result<Self, PlatformError> {
        let dirs = ProjectDirs::from("", "", "gantry").ok_or(PlatformError::NoHomeDir)?;
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_local_dir().to_path_buf(),
        })
    }

    pub fn new(config_dir: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("gantry.db")
    }

    /// Creates the data directory with user-only permissions.
    pub fn ensure_data_dir(&self) -> Result<(), PlatformError> {
        create_private_dir(&self.data_dir)
    }
}

/// Restricts the directory only if Gantry creates it: `--data-dir ~` must
/// not change the permissions of a directory the user already has.
fn create_private_dir(dir: &Path) -> Result<(), PlatformError> {
    if dir.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| PlatformError::io(dir, e))?;
    restrict_dir(dir)
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn only_a_directory_gantry_creates_is_restricted() {
        let home = tempfile::tempdir().unwrap();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        Paths::new(home.path(), home.path())
            .ensure_data_dir()
            .unwrap();
        assert_eq!(mode(home.path()), 0o755, "existing directory left alone");

        let fresh = home.path().join("gantry");
        Paths::new(&fresh, &fresh).ensure_data_dir().unwrap();
        assert_eq!(mode(&fresh), 0o700);
    }
}
