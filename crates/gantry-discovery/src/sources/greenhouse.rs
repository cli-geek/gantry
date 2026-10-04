//! Greenhouse Job Board API.
//!
//! Endpoint: `GET https://boards-api.greenhouse.io/v1/boards/{token}/jobs?content=true`,
//! no authentication. Checked live 2026-10-03: response `{jobs: [...],
//! meta: {total}}`; each job has `id`, `title`, `absolute_url`,
//! `location.name`, `company_name`, `first_published`, `updated_at`,
//! `metadata` (company-defined, e.g. "Workplace Type") and `content`
//! (entity-escaped HTML). Boards hosted on `job-boards.eu.greenhouse.io`
//! are served by the same API host. A missing board is `404 {"status":404,
//! "error":"Job board not found"}`; `GET /v1/boards/{token}` returns the
//! board's `name`, used to verify slug probes. Sends a weak `ETag`.
//!
//! Terms: the Job Board API is Greenhouse's public, documented interface
//! for publishing job posts. `robots.txt` on the API host disallows only
//! `/embed/`, which Gantry never requests.

use gantry_core::facts::PostingFacts;
use gantry_core::{Ats, Posting, PostingKey};
use serde::Deserialize;

use super::{ParseError, job_type_from_text, unix_seconds, work_mode_from_text};
use crate::{geocode, html};

pub fn board_url(token: &str) -> String {
    format!("https://boards-api.greenhouse.io/v1/boards/{token}/jobs?content=true")
}

pub fn probe_url(slug: &str) -> String {
    format!("https://boards-api.greenhouse.io/v1/boards/{slug}")
}

#[derive(Deserialize)]
struct Board {
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    id: u64,
    title: String,
    absolute_url: String,
    location: Option<NamedLocation>,
    content: Option<String>,
    company_name: Option<String>,
    first_published: Option<String>,
    updated_at: Option<String>,
    #[serde(default)]
    metadata: Option<Vec<Metadata>>,
}

#[derive(Deserialize)]
struct NamedLocation {
    name: Option<String>,
}

#[derive(Deserialize)]
struct Metadata {
    name: String,
    value: serde_json::Value,
}

#[derive(Deserialize)]
struct BoardInfo {
    name: String,
}

/// The board's display name from `GET /v1/boards/{token}`.
pub fn parse_board_name(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<BoardInfo>(body)
        .ok()
        .map(|b| b.name)
}

pub fn parse_board(token: &str, company: &str, body: &[u8]) -> Result<Vec<Posting>, ParseError> {
    let board: Board =
        serde_json::from_slice(body).map_err(|e| ParseError::json("greenhouse", &e))?;
    Ok(board
        .jobs
        .into_iter()
        .map(|job| {
            let meta = |key: &str| {
                job.metadata.iter().flatten().find_map(|m| {
                    m.name
                        .to_ascii_lowercase()
                        .contains(key)
                        .then(|| m.value.as_str().map(str::to_owned))
                        .flatten()
                })
            };
            let locations_raw: Vec<String> = job
                .location
                .and_then(|l| l.name)
                .map(|n| n.trim().to_owned())
                .filter(|n| !n.is_empty())
                .into_iter()
                .collect();
            let title = job.title.trim().to_owned();
            let description = job
                .content
                .as_deref()
                .map(|c| html::to_text(&html::unescape(c)))
                .unwrap_or_default();
            Posting {
                key: PostingKey {
                    ats: Ats::Greenhouse,
                    board_token: token.to_owned(),
                    job_id: job.id.to_string(),
                },
                company_name: job
                    .company_name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| company.to_owned()),
                url: job.absolute_url,
                locations: geocode::locate_all(&locations_raw),
                locations_raw,
                work_mode: meta("workplace").as_deref().and_then(work_mode_from_text),
                job_type: meta("employment type")
                    .as_deref()
                    .and_then(job_type_from_text)
                    .or_else(|| job_type_from_text(&title)),
                published_at: job
                    .first_published
                    .as_deref()
                    .or(job.updated_at.as_deref())
                    .and_then(unix_seconds),
                title,
                description,
                facts: PostingFacts::default(),
            }
        })
        .collect())
}
