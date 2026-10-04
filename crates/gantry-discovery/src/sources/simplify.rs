//! `simplify_listings` feed format: the `listings.json` file maintained in
//! the SimplifyJobs GitHub repositories.
//!
//! Checked live 2026-10-03 (`New-Grad-Positions`, branch `dev`,
//! `.github/scripts/listings.json`, ~19k entries, ~3k active): an array of
//! `{id, company_name, title, locations[], url, date_posted (unix s),
//! active, is_visible, sponsorship, degrees[], category, source}`.
//! `sponsorship` is one of "Offers Sponsorship", "Does Not Offer
//! Sponsorship", "U.S. Citizenship is Required", "Other".
//!
//! Terms: public repositories read as raw files from
//! `raw.githubusercontent.com`, one request per feed per run with
//! conditional GET; that host has no robots.txt (404). The repository
//! declares no license (GitHub API, 2026-10-03), so Gantry reads the list
//! for the user's own search and never redistributes it.

use gantry_core::facts::{Fact, FactSource, Sponsorship};
use serde::Deserialize;

use super::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    pub id: String,
    pub company: String,
    pub title: String,
    pub locations: Vec<String>,
    pub url: String,
    pub posted_at: Option<i64>,
    pub sponsorship: Option<Fact<Sponsorship>>,
}

#[derive(Deserialize)]
struct Listing {
    id: String,
    company_name: String,
    title: String,
    #[serde(default)]
    locations: Vec<String>,
    url: String,
    date_posted: Option<i64>,
    active: bool,
    is_visible: Option<bool>,
    sponsorship: Option<String>,
}

/// Active, visible listings.
pub fn parse(body: &[u8]) -> Result<Vec<FeedItem>, ParseError> {
    let listings: Vec<Listing> =
        serde_json::from_slice(body).map_err(|e| ParseError::json("simplify_listings", &e))?;
    Ok(listings
        .into_iter()
        .filter(|l| l.active && l.is_visible != Some(false))
        .map(|l| {
            let sponsorship = l.sponsorship.and_then(|s| {
                let value = match s.as_str() {
                    "Offers Sponsorship" => Sponsorship::Offered,
                    "Does Not Offer Sponsorship" => Sponsorship::NotOffered,
                    "U.S. Citizenship is Required" => Sponsorship::CitizenshipRequired,
                    _ => return None,
                };
                Some(Fact {
                    value,
                    source: FactSource::Feed,
                    evidence: s,
                })
            });
            FeedItem {
                id: l.id,
                company: l.company_name.trim().to_owned(),
                title: l.title.trim().to_owned(),
                locations: l.locations,
                url: l.url,
                posted_at: l.date_posted,
                sponsorship,
            }
        })
        .collect())
}
