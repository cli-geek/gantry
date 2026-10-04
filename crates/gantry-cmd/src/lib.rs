//! The command layer (§3.4). One typed function per operation; the CLI and
//! the app call these and nothing else. Every output type serializes to
//! JSON matching a schema published under `docs/schema/`.

mod discover;
mod doctor;
mod error;
mod postings;
pub mod schema;
mod urls;

use gantry_store::{ConfigSnapshot, Store, load_snapshot};

pub use discover::{run_discover, run_discover_with};
pub use doctor::{CheckStatus, DoctorCheck, DoctorReport, Services, doctor};
pub use error::{CmdError, ErrorBody, ErrorOutput, ExitCode};
pub use gantry_core::filter::{Check, FilterName, Verdict};
pub use gantry_discovery::DiscoverReport;
pub use gantry_platform::Paths;
pub use postings::{PostingStatus, PostingSummary, PostingsQuery, StatusFilter, list_postings};
pub use urls::{UrlAdded, add_url};

/// Where a command reads and writes. Built once per process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub paths: Paths,
}

impl Context {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    pub(crate) fn snapshot(&self) -> Result<ConfigSnapshot, CmdError> {
        load_snapshot(self.paths.config_dir()).map_err(CmdError::Config)
    }

    /// Opens the database, creating the data directory (user-only
    /// permissions) and running migrations on first use.
    pub(crate) fn open_store(&self) -> Result<Store, CmdError> {
        self.paths.ensure_data_dir()?;
        Ok(Store::open(&self.paths.db_path())?)
    }
}

/// Current time in Unix seconds; the one clock commands read.
pub fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}
