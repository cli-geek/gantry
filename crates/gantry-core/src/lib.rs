//! Domain types and the deterministic parts of the pipeline: posting model,
//! search profile (`search.toml`), fact detection, and filters (§4.2–4.3).
//!
//! Nothing here performs I/O. Nothing here knows about any particular
//! occupation: every value a filter compares against comes from the user's
//! search profile or verified bank.

pub mod ats;
pub mod bank;
pub mod facts;
pub mod filter;
pub mod geo;
pub mod location;
pub mod posting;
pub mod search;
pub mod text;

pub use ats::{Ats, PostingKey};
pub use posting::{JobType, Pay, PayPeriod, Posting, WorkMode};
