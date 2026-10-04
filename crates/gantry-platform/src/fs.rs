use std::io::Write;
use std::path::Path;

use crate::{PlatformError, os};

/// Replaces `path` with `contents` so a reader sees either the old file or
/// the new one, never a partial write: write a temp file in the same
/// directory, fsync it, rename it over the target, then fsync the directory.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), PlatformError> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let file_name = path
        .file_name()
        .ok_or_else(|| PlatformError::io(path, std::io::ErrorKind::InvalidInput.into()))?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    let result = (|| {
        let mut file = std::fs::File::create(&tmp).map_err(|e| PlatformError::io(&tmp, e))?;
        restrict_file(&tmp)?;
        file.write_all(contents)
            .map_err(|e| PlatformError::io(&tmp, e))?;
        file.sync_all().map_err(|e| PlatformError::io(&tmp, e))?;
        drop(file);
        os::replace_file(&tmp, path)?;
        os::sync_dir(dir)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Makes a directory readable only by the current user (§12).
pub fn restrict_dir(path: &Path) -> Result<(), PlatformError> {
    os::restrict_dir(path)
}

/// Makes a file readable only by the current user (§12).
pub fn restrict_file(path: &Path) -> Result<(), PlatformError> {
    os::restrict_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_replaces_contents_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search.toml");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("search.toml")]);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_creates_user_only_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legal.toml");
        write_atomic(&path, b"x").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
