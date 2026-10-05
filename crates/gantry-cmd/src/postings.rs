use gantry_core::Ats;
use gantry_core::filter::Check;
use gantry_store::ListFilter;
use schemars::JsonSchema;
use serde::Serialize;

use crate::{CmdError, Context};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusFilter {
    #[default]
    Passed,
    FilteredOut,
    All,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PostingsQuery {
    pub status: StatusFilter,
    pub include_closed: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostingStatus {
    Passed,
    FilteredOut,
    NotEvaluated,
}

/// One row of the list view (§4.3: filtered-out postings show reasons).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PostingSummary {
    pub id: i64,
    pub ats: Ats,
    pub company: String,
    pub title: String,
    pub url: String,
    pub location: String,
    pub published_at: Option<i64>,
    pub first_seen: i64,
    pub closed_at: Option<i64>,
    /// The earlier posting this one reposts.
    pub duplicate_of: Option<i64>,
    pub status: PostingStatus,
    pub checks: Vec<Check>,
    /// Every source that listed it: an ATS, `feed:<id>`, `hn`, `manual`.
    pub sources: Vec<String>,
}

pub fn list_postings(ctx: &Context, query: PostingsQuery) -> Result<Vec<PostingSummary>, CmdError> {
    if query.limit.is_some_and(|n| i64::try_from(n).is_err()) {
        return Err(CmdError::InvalidInput(format!(
            "--limit must be at most {}",
            i64::MAX
        )));
    }
    let store = ctx.open_store()?;
    let rows = store.list_postings(&ListFilter {
        passed: match query.status {
            StatusFilter::Passed => Some(true),
            StatusFilter::FilteredOut => Some(false),
            StatusFilter::All => None,
        },
        include_closed: query.include_closed,
        limit: query.limit,
    })?;
    Ok(rows
        .into_iter()
        .map(|r| PostingSummary {
            id: r.id,
            ats: r.ats,
            company: r.company_name,
            title: r.title,
            url: r.url,
            location: r.location_raw,
            published_at: r.published_at,
            first_seen: r.first_seen,
            closed_at: r.closed_at,
            duplicate_of: r.duplicate_of,
            status: match r.passed {
                Some(true) => PostingStatus::Passed,
                Some(false) => PostingStatus::FilteredOut,
                None => PostingStatus::NotEvaluated,
            },
            checks: r.checks,
            sources: r.sources,
        })
        .collect())
}
