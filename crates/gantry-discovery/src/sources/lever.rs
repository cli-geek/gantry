//! Lever Postings API.
//!
//! Endpoint: `GET https://api.lever.co/v0/postings/{site}?mode=json`
//! (EU: `api.eu.lever.co`), no authentication. Checked live 2026-10-03:
//! a JSON array; each posting has `id`, `text` (title), `hostedUrl`,
//! `categories.{commitment, location, allLocations, team}`, `createdAt`
//! (ms), `workplaceType` (`onsite`/`hybrid`/`remote`/`unspecified`),
//! `descriptionPlain`, `lists[{text, content}]`, `additionalPlain`, and an
//! optional `salaryRange {min, max, currency, interval}`. A board with no
//! openings is `[]`; a missing one is `404 {"ok":false,"error":"Document
//! not found"}`. The API does not return the company name. Sends `ETag`.
//!
//! Terms: the Postings API is Lever's documented public interface for
//! job sites. `robots.txt` on the API host allows `/` with a 1 s crawl
//! delay, which the fetcher honors.

use gantry_core::facts::{Fact, FactSource, PostingFacts};
use gantry_core::{Ats, Pay, Posting, PostingKey};
use serde::Deserialize;

use super::{ParseError, job_type_from_text, pay_period, work_mode_from_text};
use crate::{geocode, html};

pub fn board_url(ats: Ats, site: &str) -> String {
    let host = if ats == Ats::LeverEu {
        "api.eu.lever.co"
    } else {
        "api.lever.co"
    };
    format!("https://{host}/v0/postings/{site}?mode=json")
}

pub fn probe_url(slug: &str) -> String {
    format!("https://api.lever.co/v0/postings/{slug}?mode=json&limit=1")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeverPosting {
    id: String,
    text: String,
    hosted_url: String,
    categories: Option<Categories>,
    created_at: Option<i64>,
    workplace_type: Option<String>,
    description_plain: Option<String>,
    #[serde(default)]
    lists: Vec<List>,
    additional_plain: Option<String>,
    salary_range: Option<SalaryRange>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Categories {
    commitment: Option<String>,
    location: Option<String>,
    #[serde(default)]
    all_locations: Vec<String>,
}

#[derive(Deserialize)]
struct List {
    text: String,
    content: String,
}

#[derive(Deserialize)]
struct SalaryRange {
    min: Option<f64>,
    max: Option<f64>,
    currency: Option<String>,
    interval: Option<String>,
}

pub fn parse_board(
    ats: Ats,
    site: &str,
    company: &str,
    body: &[u8],
) -> Result<Vec<Posting>, ParseError> {
    let postings: Vec<LeverPosting> =
        serde_json::from_slice(body).map_err(|e| ParseError::json("lever", &e))?;
    Ok(postings
        .into_iter()
        .map(|p| {
            let categories = p.categories.unwrap_or(Categories {
                commitment: None,
                location: None,
                all_locations: Vec::new(),
            });
            let locations_raw: Vec<String> = if categories.all_locations.is_empty() {
                categories.location.into_iter().collect()
            } else {
                categories.all_locations
            };
            let mut description = p.description_plain.unwrap_or_default();
            for list in &p.lists {
                description.push_str(&format!(
                    "\n{}\n{}",
                    list.text,
                    html::to_text(&list.content)
                ));
            }
            if let Some(extra) = p.additional_plain {
                description.push('\n');
                description.push_str(&extra);
            }
            let title = p.text.trim().to_owned();
            let pay = p.salary_range.and_then(|s| {
                let period = pay_period(s.interval.as_deref()?)?;
                (s.min.is_some() || s.max.is_some()).then(|| Fact {
                    evidence: format!(
                        "{}-{} {} {}",
                        s.min.unwrap_or_default(),
                        s.max.unwrap_or_default(),
                        s.currency.as_deref().unwrap_or("USD"),
                        s.interval.as_deref().unwrap_or_default()
                    ),
                    value: Pay {
                        min: s.min,
                        max: s.max,
                        currency: s.currency.unwrap_or_else(|| "USD".into()),
                        period,
                    },
                    source: FactSource::Ats,
                })
            });
            Posting {
                key: PostingKey {
                    ats,
                    board_token: site.to_owned(),
                    job_id: p.id.to_ascii_lowercase(),
                },
                company_name: company.to_owned(),
                url: p.hosted_url,
                locations: geocode::locate_all(&locations_raw),
                locations_raw,
                work_mode: p.workplace_type.as_deref().and_then(work_mode_from_text),
                job_type: categories
                    .commitment
                    .as_deref()
                    .and_then(job_type_from_text)
                    .or_else(|| job_type_from_text(&title)),
                published_at: p.created_at.map(|ms| ms / 1000),
                title,
                description: description.trim().to_owned(),
                facts: PostingFacts {
                    pay,
                    ..PostingFacts::default()
                },
            }
        })
        .collect())
}
