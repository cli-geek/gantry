//! Gantry's two stores (§9): the TOML profile in the config directory and
//! the SQLite run-state database in the data directory.

mod bank;
mod config;
mod db;
mod error;
mod postings;
mod questions;

pub use bank::{BankDocs, load_bank_docs, update_table};
pub use config::{
    CompanyEntry, CompanyList, ConfigFile, ConfigSnapshot, DiscoverySettings, FileState, Settings,
    check_files, load_snapshot, save_toml,
};
pub use db::{CompanyRow, HttpValidators, Store};
pub use error::StoreError;
pub use postings::{ListFilter, PostingRow, StoredPosting, UpsertOutcome, UpsertStatus};
pub use questions::bundled_catalog;
