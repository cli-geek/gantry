//! Hacker News "Ask HN: Who is hiring?" threads, through the official API.
//!
//! Endpoints: `GET https://hacker-news.firebaseio.com/v0/user/whoishiring.json`
//! (`submitted`: newest first) and `GET /v0/item/{id}.json`. Checked live
//! 2026-10-03: the October 2026 thread is a `story` titled "Ask HN: Who is
//! hiring? (October 2026)" whose `kids` are the top-level comments; each
//! comment has `text` as HTML, conventionally starting
//! "Company | Role | Location | ...".
//!
//! Terms: the API is public and documented, with no rate limit stated;
//! `robots.txt` allows only `*.json` paths, which is all Gantry requests.
//! Comments are free text: M1 takes only the company name (for slug
//! probing) and links (for the resolver). Turning a comment into a posting
//! needs quarantined extraction (§4.4, M3).

use serde::Deserialize;

use super::ParseError;
use crate::html;

pub const USER_URL: &str = "https://hacker-news.firebaseio.com/v0/user/whoishiring.json";

pub fn item_url(id: u64) -> String {
    format!("https://hacker-news.firebaseio.com/v0/item/{id}.json")
}

/// How many of the account's newest submissions to scan for the thread.
/// It posts three threads a month, so the current one is within these.
pub const THREAD_SEARCH_DEPTH: usize = 6;

#[derive(Deserialize)]
struct User {
    #[serde(default)]
    submitted: Vec<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Item {
    pub id: u64,
    pub title: Option<String>,
    #[serde(default)]
    pub kids: Vec<u64>,
    pub text: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub dead: bool,
}

/// A company name and links found in one comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lead {
    pub company: Option<String>,
    pub urls: Vec<String>,
}

pub fn parse_user(body: &[u8]) -> Result<Vec<u64>, ParseError> {
    serde_json::from_slice::<User>(body)
        .map(|u| u.submitted)
        .map_err(|e| ParseError::json("hacker_news", &e))
}

/// `Ok(None)` for `null`, which the API returns for a missing item.
pub fn parse_item(body: &[u8]) -> Result<Option<Item>, ParseError> {
    serde_json::from_slice(body).map_err(|e| ParseError::json("hacker_news", &e))
}

pub fn is_hiring_thread(item: &Item) -> bool {
    item.title
        .as_deref()
        .is_some_and(|t| t.starts_with("Ask HN: Who is hiring?"))
}

/// The first `|`-separated field of the comment's first line is the
/// company by convention; it is kept only when it looks like a name.
pub fn parse_comment(item: &Item) -> Option<Lead> {
    if item.deleted || item.dead {
        return None;
    }
    let text = item.text.as_deref()?;
    let first_line_html = text.split("<p>").next().unwrap_or(text);
    let first_line = html::to_text(first_line_html);
    let company = first_line
        .split('|')
        .next()
        .map(str::trim)
        .filter(|c| {
            !c.is_empty()
                && c.len() <= 60
                && first_line.contains('|')
                && !c.contains("://")
                && !c.starts_with("www.")
        })
        .map(str::to_owned);
    Some(Lead {
        company,
        urls: html::links(text),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_yields_company_and_links() {
        let item = Item {
            id: 1,
            text: Some(
                "Pinecrest Robotics | Firmware Engineer | Boulder, CO | ONSITE<p>Apply: \
                 <a href=\"https:&#x2F;&#x2F;jobs.lever.co&#x2F;pinecrest\" rel=\"nofollow\">here</a>"
                    .into(),
            ),
            ..Item::default()
        };
        let lead = parse_comment(&item).unwrap();
        assert_eq!(lead.company.as_deref(), Some("Pinecrest Robotics"));
        assert_eq!(lead.urls, ["https://jobs.lever.co/pinecrest"]);
    }

    #[test]
    fn free_text_first_line_has_no_company() {
        let item = Item {
            id: 2,
            text: Some("We are hiring engineers in many places.<p>Email us.".into()),
            ..Item::default()
        };
        assert_eq!(parse_comment(&item).unwrap().company, None);
        let dead = Item { dead: true, ..item };
        assert!(parse_comment(&dead).is_none());
    }
}
