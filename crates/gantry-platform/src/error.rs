use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0} is not implemented on {os} yet", os = crate::os::NAME)]
    Unsupported(&'static str),
    #[error("could not determine the home directory; set GANTRY_CONFIG_DIR and GANTRY_DATA_DIR")]
    NoHomeDir,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("secret store: {0}")]
    Secret(#[from] keyring_core::Error),
    #[error("`{command}` failed: {detail}")]
    Command { command: String, detail: String },
}

impl PlatformError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
