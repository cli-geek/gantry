use schemars::JsonSchema;
use serde::Serialize;

/// Process exit codes. Stable: scripts and agents depend on them
/// (`docs/cli.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    Ok = 0,
    /// The command failed (database, file system, unexpected error).
    Error = 1,
    /// Bad arguments or input; nothing was changed.
    Usage = 2,
    /// `doctor` found at least one failing check.
    ChecksFailed = 3,
    /// The operation belongs to a later milestone.
    NotAvailable = 4,
    /// A config file is missing a required value or does not parse.
    Config = 5,
    /// Another discovery run holds the run lock; nothing was changed.
    Busy = 6,
}

#[derive(Debug, thiserror::Error)]
pub enum CmdError {
    #[error("{0}")]
    Config(gantry_store::StoreError),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{what} is not available yet; it arrives in milestone {milestone}")]
    NotAvailable {
        what: &'static str,
        milestone: &'static str,
    },
    #[error(transparent)]
    Store(#[from] gantry_store::StoreError),
    #[error(transparent)]
    Platform(#[from] gantry_platform::PlatformError),
    #[error("{0}")]
    Network(String),
    #[error("another discovery run is in progress")]
    Busy,
}

impl CmdError {
    /// Stable machine-readable error kind for `--json` output.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Config(_) => "config",
            Self::InvalidInput(_) => "invalid_input",
            Self::NotAvailable { .. } => "not_available",
            Self::Store(_) => "store",
            Self::Platform(_) => "platform",
            Self::Network(_) => "network",
            Self::Busy => "busy",
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Config(_) => ExitCode::Config,
            Self::InvalidInput(_) => ExitCode::Usage,
            Self::NotAvailable { .. } => ExitCode::NotAvailable,
            Self::Store(_) | Self::Platform(_) | Self::Network(_) => ExitCode::Error,
            Self::Busy => ExitCode::Busy,
        }
    }

    pub fn to_output(&self) -> ErrorOutput {
        ErrorOutput {
            error: ErrorBody {
                kind: self.kind().to_owned(),
                message: self.to_string(),
            },
        }
    }
}

/// What every command prints on stdout with `--json` when it fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ErrorOutput {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ErrorBody {
    /// `config`, `invalid_input`, `not_available`, `store`, `platform`,
    /// `network`, `busy` or `usage`.
    pub kind: String,
    pub message: String,
}
