//! JSON Schemas for every `--json` output (§3.4). The files under
//! `docs/schema/` are generated from these types; a test fails when they
//! drift (`GANTRY_UPDATE_SCHEMAS=1 cargo test -p gantry-cmd` rewrites them).

use schemars::{Schema, schema_for};

use crate::{DiscoverReport, DoctorReport, ErrorOutput, PostingSummary, UrlAdded};

/// `(name, schema)` for each output, named after the command.
pub fn all() -> Vec<(&'static str, Schema)> {
    vec![
        ("doctor", schema_for!(DoctorReport)),
        ("run", schema_for!(DiscoverReport)),
        ("postings-list", schema_for!(Vec<PostingSummary>)),
        ("url-add", schema_for!(UrlAdded)),
        ("error", schema_for!(ErrorOutput)),
    ]
}

pub fn get(name: &str) -> Option<Schema> {
    all().into_iter().find(|(n, _)| *n == name).map(|(_, s)| s)
}

/// Names `get` accepts.
pub fn names() -> Vec<&'static str> {
    all().into_iter().map(|(n, _)| n).collect()
}
