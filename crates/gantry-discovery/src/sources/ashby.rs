//! Ashby Job Postings API.
//!
//! Endpoint: `GET https://api.ashbyhq.com/posting-api/job-board/{name}?includeCompensation=true`,
//! no authentication. Checked live 2026-10-03: `{apiVersion, jobs: [...]}`;
//! each job has `id`, `title`, `location`, `secondaryLocations[{location}]`,
//! `isListed`, `isRemote`, `workplaceType` (`OnSite`/`Hybrid`/`Remote` or
//! null), `employmentType` (`FullTime`, `Contract`, …), `publishedAt`,
//! `jobUrl`, `descriptionPlain`, and `compensation.summaryComponents[
//! {compensationType, interval, currencyCode, minValue, maxValue}]`. No
//! application form fields (see §4.1.2). A missing board is `404 Not
//! Found`; an empty one is `{"jobs":[]}`. Sends a weak `ETag`.
//!
//! Terms: the posting API is Ashby's documented public interface for
//! career sites. `robots.txt` on the API host answers 401, which RFC 9309
//! treats as no restrictions.

use gantry_core::facts::{Fact, FactSource, PostingFacts};
use gantry_core::{Ats, Pay, Posting, PostingKey};
use serde::Deserialize;

use super::{ParseError, job_type_from_text, pay_period, unix_seconds, work_mode_from_text};
use crate::geocode;

pub fn board_url(name: &str) -> String {
    format!("https://api.ashbyhq.com/posting-api/job-board/{name}?includeCompensation=true")
}

pub fn probe_url(slug: &str) -> String {
    format!("https://api.ashbyhq.com/posting-api/job-board/{slug}")
}

#[derive(Deserialize)]
struct Board {
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    title: String,
    location: Option<String>,
    #[serde(default)]
    secondary_locations: Vec<Secondary>,
    is_listed: Option<bool>,
    is_remote: Option<bool>,
    workplace_type: Option<String>,
    employment_type: Option<String>,
    published_at: Option<String>,
    job_url: String,
    description_plain: Option<String>,
    compensation: Option<Compensation>,
}

#[derive(Deserialize)]
struct Secondary {
    location: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Compensation {
    #[serde(default)]
    summary_components: Vec<Component>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Component {
    compensation_type: String,
    interval: Option<String>,
    currency_code: Option<String>,
    min_value: Option<f64>,
    max_value: Option<f64>,
}

pub fn parse_board(name: &str, company: &str, body: &[u8]) -> Result<Vec<Posting>, ParseError> {
    let board: Board = serde_json::from_slice(body).map_err(|e| ParseError::json("ashby", &e))?;
    Ok(board
        .jobs
        .into_iter()
        .filter(|j| j.is_listed != Some(false))
        .map(|j| {
            let locations_raw: Vec<String> = j
                .location
                .into_iter()
                .chain(j.secondary_locations.into_iter().map(|s| s.location))
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .collect();
            let pay = j.compensation.and_then(|c| {
                c.summary_components
                    .into_iter()
                    .filter(|s| matches!(s.compensation_type.as_str(), "Salary" | "Hourly"))
                    .find_map(|s| {
                        let period = pay_period(s.interval.as_deref()?)?;
                        (s.min_value.is_some() || s.max_value.is_some()).then(|| Fact {
                            evidence: format!(
                                "{} {}-{} {}",
                                s.compensation_type,
                                s.min_value.unwrap_or_default(),
                                s.max_value.unwrap_or_default(),
                                s.currency_code.as_deref().unwrap_or("USD")
                            ),
                            value: Pay {
                                min: s.min_value,
                                max: s.max_value,
                                currency: s.currency_code.unwrap_or_else(|| "USD".into()),
                                period,
                            },
                            source: FactSource::Ats,
                        })
                    })
            });
            let title = j.title.trim().to_owned();
            let work_mode = j
                .workplace_type
                .as_deref()
                .and_then(work_mode_from_text)
                .or_else(|| (j.is_remote == Some(true)).then_some(gantry_core::WorkMode::Remote));
            Posting {
                key: PostingKey {
                    ats: Ats::Ashby,
                    board_token: name.to_owned(),
                    job_id: j.id.to_ascii_lowercase(),
                },
                company_name: company.to_owned(),
                url: j.job_url,
                locations: geocode::locate_all(&locations_raw),
                locations_raw,
                work_mode,
                job_type: j
                    .employment_type
                    .as_deref()
                    .and_then(job_type_from_text)
                    .or_else(|| job_type_from_text(&title)),
                published_at: j.published_at.as_deref().and_then(unix_seconds),
                title,
                description: j.description_plain.unwrap_or_default(),
                facts: PostingFacts {
                    pay,
                    ..PostingFacts::default()
                },
            }
        })
        .collect())
}
