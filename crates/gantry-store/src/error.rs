use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database migration: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("stored JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{path}: {message}")]
    Toml { path: PathBuf, message: String },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Platform(#[from] gantry_platform::PlatformError),
    /// A mutex guarding the connection was poisoned by a panic elsewhere.
    #[error("database connection is unusable after an earlier panic")]
    Poisoned,
}
