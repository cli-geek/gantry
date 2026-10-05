//! Manual URL resolver (§4.1): turns a posting or board URL into the board
//! API it belongs to. Pure; makes no requests.

use gantry_core::Ats;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Resolved {
    /// One posting on a board API.
    Posting {
        ats: Ats,
        board_token: String,
        job_id: String,
    },
    /// A whole board.
    Board { ats: Ats, board_token: String },
    /// A Greenhouse board embedded in a company site (`?gh_jid=`). The board
    /// token is not in the URL; it is found by probing the host's name.
    GreenhouseEmbed { host: String, job_id: String },
    /// Any other site. Discovered and listed, but nothing to poll.
    External { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an http(s) URL: {0}")]
pub struct ResolveError(pub String);

/// Query parameters that only track where a click came from.
const TRACKING_PARAMS: &[&str] = &[
    "ref",
    "src",
    "source",
    "gh_src",
    "lever-source",
    "lever-origin",
];

pub fn resolve(input: &str) -> Result<Resolved, ResolveError> {
    let url = Url::parse(input.trim()).map_err(|_| ResolveError(input.to_owned()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ResolveError(input.to_owned()));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let segments: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(percent_decode).collect())
        .unwrap_or_default();
    let seg = |i: usize| segments.get(i).map(String::as_str);
    let query = |name: &str| {
        url.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };

    let resolved = match host.as_str() {
        "boards.greenhouse.io"
        | "job-boards.greenhouse.io"
        | "boards.eu.greenhouse.io"
        | "job-boards.eu.greenhouse.io" => match (seg(0), seg(1), seg(2)) {
            (Some("embed"), ..) => match (query("for"), query("token")) {
                (Some(board), Some(job)) if valid_token(&board) && is_digits(&job) => {
                    posting(Ats::Greenhouse, &board, &job)
                }
                (Some(board), None) if valid_token(&board) => board_of(Ats::Greenhouse, &board),
                _ => external(&url),
            },
            (Some(board), Some("jobs"), Some(job)) if valid_token(board) && is_digits(job) => {
                posting(Ats::Greenhouse, board, job)
            }
            (Some(board), None, None) if valid_token(board) => board_of(Ats::Greenhouse, board),
            _ => external(&url),
        },
        "jobs.lever.co" | "jobs.eu.lever.co" => {
            let ats = if host.contains(".eu.") {
                Ats::LeverEu
            } else {
                Ats::Lever
            };
            match (seg(0), seg(1)) {
                (Some(site), Some(job)) if valid_token(site) && is_uuid(job) => {
                    posting(ats, site, job)
                }
                (Some(site), None) if valid_token(site) => board_of(ats, site),
                _ => external(&url),
            }
        }
        "jobs.ashbyhq.com" => match (seg(0), seg(1)) {
            (Some(name), Some(job)) if valid_token(name) && is_uuid(job) => {
                posting(Ats::Ashby, name, job)
            }
            (Some(name), None) if valid_token(name) => board_of(Ats::Ashby, name),
            _ => external(&url),
        },
        _ => match query("gh_jid") {
            Some(job) if is_digits(&job) => Resolved::GreenhouseEmbed { host, job_id: job },
            _ => external(&url),
        },
    };
    Ok(resolved)
}

fn posting(ats: Ats, board: &str, job: &str) -> Resolved {
    Resolved::Posting {
        ats,
        board_token: ats.canonical_token(board),
        job_id: job.to_ascii_lowercase(),
    }
}

fn board_of(ats: Ats, board: &str) -> Resolved {
    Resolved::Board {
        ats,
        board_token: ats.canonical_token(board),
    }
}

/// The URL without its fragment and tracking parameters, so the same
/// page reached from different sources compares equal.
fn external(url: &Url) -> Resolved {
    let mut clean = url.clone();
    clean.set_fragment(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_") && !TRACKING_PARAMS.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        clean.set_query(None);
    } else {
        clean.query_pairs_mut().clear().extend_pairs(kept);
    }
    Resolved::External {
        url: clean.to_string(),
    }
}

fn percent_decode(s: &str) -> String {
    let hex = |b: u8| char::from(b).to_digit(16);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            // Two hex digits always fit in a byte.
            out.push(u8::try_from(hi * 16 + lo).unwrap_or(b'?'));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Board tokens seen on all three ATSes: letters, digits, `-`, `_`, `.`,
/// and spaces in some Ashby names.
fn valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(url: &str) -> Resolved {
        resolve(url).unwrap()
    }

    #[test]
    fn greenhouse_forms() {
        let want = posting(Ats::Greenhouse, "northwindlabs", "4012345");
        assert_eq!(
            r("https://job-boards.greenhouse.io/northwindlabs/jobs/4012345"),
            want
        );
        assert_eq!(
            r("https://boards.greenhouse.io/northwindlabs/jobs/4012345?gh_src=x"),
            want
        );
        assert_eq!(
            r("https://job-boards.eu.greenhouse.io/northwindlabs/jobs/4012345"),
            want
        );
        assert_eq!(
            r("https://boards.greenhouse.io/embed/job_app?for=northwindlabs&token=4012345"),
            want
        );
        assert_eq!(
            r("https://job-boards.greenhouse.io/northwindlabs"),
            board_of(Ats::Greenhouse, "northwindlabs")
        );
    }

    #[test]
    fn embedded_greenhouse_on_company_site() {
        assert_eq!(
            r("https://careers.northwind.example/positions/8184174?gh_jid=8184174"),
            Resolved::GreenhouseEmbed {
                host: "careers.northwind.example".into(),
                job_id: "8184174".into()
            }
        );
    }

    #[test]
    fn lever_and_ashby_forms() {
        let job = "6ed76ce8-4156-4b60-b120-403538bd66cd";
        assert_eq!(
            r(&format!("https://jobs.lever.co/quillstack/{job}/apply")),
            posting(Ats::Lever, "quillstack", job)
        );
        assert_eq!(
            r(&format!("https://jobs.eu.lever.co/quillstack/{job}")),
            posting(Ats::LeverEu, "quillstack", job)
        );
        assert_eq!(
            r(&format!(
                "https://jobs.ashbyhq.com/lumenforge/{job}/application?utm_source=x"
            )),
            posting(Ats::Ashby, "lumenforge", job)
        );
        assert_eq!(
            r("https://jobs.ashbyhq.com/Lumen%20Forge"),
            board_of(Ats::Ashby, "Lumen Forge")
        );
    }

    #[test]
    fn external_urls_lose_tracking() {
        assert_eq!(
            r(
                "https://careers.tidewater.example/jobs/77?utm_source=Simplify&ref=Simplify&lang=en#apply"
            ),
            Resolved::External {
                url: "https://careers.tidewater.example/jobs/77?lang=en".into()
            }
        );
        assert_eq!(
            r("https://jobs.lever.co/"),
            Resolved::External {
                url: "https://jobs.lever.co/".into()
            }
        );
    }

    #[test]
    fn rejects_non_http() {
        assert!(resolve("mailto:jobs@example.com").is_err());
        assert!(resolve("not a url").is_err());
    }
}
