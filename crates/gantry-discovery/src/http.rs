//! The only way discovery talks to the network, with the §4.1 politeness
//! rules built in: one request per second per host (or the host's
//! `Crawl-delay`), robots.txt honored for every host, conditional GET,
//! and exponential backoff on 429 and 5xx.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gantry_store::{HttpValidators, Store, StoreError};
use tokio::sync::OnceCell;
use tokio::time::Instant;
use url::Url;

use crate::robots::Robots;

/// Identifies Gantry to every host. The project URL is added once the
/// repository is public.
pub const USER_AGENT: &str = concat!(
    "Gantry/",
    env!("CARGO_PKG_VERSION"),
    " (open-source job search assistant; one user, polite polling)"
);

/// Product token matched against robots.txt `User-agent` lines.
const ROBOTS_AGENT: &str = "gantry";

/// Responses larger than this are refused. The largest board seen live
/// (2026-10) is about 15 MB.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

/// Sites whose terms prohibit automated access (plan §11.3, §4.1.1). Gantry
/// never sends them a request, whatever the caller asks; their postings go
/// through manual paste mode.
const EXCLUDED_DOMAINS: &[&str] = &[
    "myworkdayjobs.com",
    "workday.com",
    "joinhandshake.com",
    "linkedin.com",
    "indeed.com",
    "glassdoor.com",
    "ziprecruiter.com",
    "wellfound.com",
];

fn is_excluded_host(host: &str) -> bool {
    EXCLUDED_DOMAINS
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// Names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

pub type TransportFuture<'a> =
    Pin<Box<dyn Future<Output = Result<HttpResponse, String>> + Send + 'a>>;

/// Sends one GET. `Err` is a network-level failure; any HTTP status is
/// `Ok`. Swapped for saved fixtures in tests.
pub trait Transport: Send + Sync + std::fmt::Debug {
    fn get(&self, request: HttpRequest) -> TransportFuture<'_>;
}

#[derive(Debug)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
}

impl Transport for ReqwestTransport {
    fn get(&self, request: HttpRequest) -> TransportFuture<'_> {
        Box::pin(async move {
            let mut builder = self.client.get(&request.url);
            for (name, value) in &request.headers {
                builder = builder.header(name, value);
            }
            let mut resp = builder.send().await.map_err(|e| e.to_string())?;
            let status = resp.status().as_u16();
            let headers = resp
                .headers()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
                .collect();
            let mut body = Vec::new();
            while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
                if body.len() + chunk.len() > MAX_BODY_BYTES {
                    return Err(format!("response larger than {MAX_BODY_BYTES} bytes"));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Politeness {
    /// Minimum spacing between requests to one host.
    pub min_interval: Duration,
    /// First retry wait; doubles each attempt.
    pub retry_base: Duration,
    pub max_attempts: u32,
}

impl Default for Politeness {
    fn default() -> Self {
        Self {
            min_interval: Duration::from_secs(1),
            retry_base: Duration::from_secs(2),
            max_attempts: 4,
        }
    }
}

#[derive(Debug)]
pub enum Fetched {
    Body {
        body: Vec<u8>,
        /// Commit with [`Fetcher::commit`] once the body is processed, so
        /// a failure while processing does not turn the next run into a
        /// 304 that skips the data.
        validators: HttpValidators,
    },
    NotModified,
    NotFound,
    /// robots.txt forbids the path; nothing was requested.
    Disallowed,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("invalid URL {0}")]
    InvalidUrl(String),
    #[error("{url}: {message}")]
    Transport { url: String, message: String },
    #[error("{url}: HTTP {status}")]
    Status { url: String, status: u16 },
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Default)]
struct HostState {
    next_slot: tokio::sync::Mutex<Option<Instant>>,
    robots: OnceCell<Robots>,
}

#[derive(Debug)]
pub struct Fetcher<'a> {
    transport: &'a dyn Transport,
    store: &'a Store,
    politeness: Politeness,
    hosts: Mutex<HashMap<String, Arc<HostState>>>,
    now: i64,
}

impl<'a> Fetcher<'a> {
    pub fn new(
        transport: &'a dyn Transport,
        store: &'a Store,
        politeness: Politeness,
        now: i64,
    ) -> Self {
        Self {
            transport,
            store,
            politeness,
            hosts: Mutex::new(HashMap::new()),
            now,
        }
    }

    fn host(&self, host: &str) -> Arc<HostState> {
        // A poisoned map only means another task panicked mid-insert; the
        // map itself is still usable.
        let mut hosts = self
            .hosts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(hosts.entry(host.to_owned()).or_default())
    }

    /// Waits for this host's next request slot and reserves the one after.
    async fn wait_turn(&self, state: &HostState) {
        let delay = state
            .robots
            .get()
            .and_then(Robots::crawl_delay)
            .map_or(self.politeness.min_interval, |d| {
                d.max(self.politeness.min_interval)
            });
        let mut next = state.next_slot.lock().await;
        let now = Instant::now();
        let at = next.map_or(now, |n| n.max(now));
        *next = Some(at + delay);
        drop(next);
        tokio::time::sleep_until(at).await;
    }

    async fn robots(&self, url: &Url, state: &HostState) -> Robots {
        state
            .robots
            .get_or_init(|| async {
                let mut robots_url = url.clone();
                robots_url.set_path("/robots.txt");
                robots_url.set_query(None);
                self.wait_turn(state).await;
                let request = HttpRequest {
                    url: robots_url.to_string(),
                    headers: Vec::new(),
                };
                match self.transport.get(request).await {
                    Ok(r) if (200..300).contains(&r.status) => {
                        Robots::parse(&String::from_utf8_lossy(&r.body), ROBOTS_AGENT)
                    }
                    // RFC 9309: a 4xx means no robots.txt, so no limits.
                    Ok(r) if (400..500).contains(&r.status) => Robots::allow_all(),
                    // Unreachable or 5xx: assume everything is disallowed.
                    _ => Robots::disallow_all(),
                }
            })
            .await
            .clone()
    }

    /// GETs `url` politely. With `conditional`, sends stored validators and
    /// may return [`Fetched::NotModified`].
    pub async fn get(&self, url: &str, conditional: bool) -> Result<Fetched, FetchError> {
        let parsed = Url::parse(url).map_err(|_| FetchError::InvalidUrl(url.to_owned()))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| FetchError::InvalidUrl(url.to_owned()))?
            .to_ascii_lowercase();
        if is_excluded_host(&host) {
            return Ok(Fetched::Disallowed);
        }
        let state = self.host(&host);
        let robots = self.robots(&parsed, &state).await;
        let path = match parsed.query() {
            Some(q) => format!("{}?{q}", parsed.path()),
            None => parsed.path().to_owned(),
        };
        if !robots.allows(&path) {
            return Ok(Fetched::Disallowed);
        }

        let mut headers = Vec::new();
        if conditional && let Some(v) = self.store.http_validators(url)? {
            if let Some(etag) = v.etag {
                headers.push(("if-none-match".to_owned(), etag));
            }
            if let Some(lm) = v.last_modified {
                headers.push(("if-modified-since".to_owned(), lm));
            }
        }

        let mut last_error = String::new();
        for attempt in 0..self.politeness.max_attempts {
            self.wait_turn(&state).await;
            let request = HttpRequest {
                url: url.to_owned(),
                headers: headers.clone(),
            };
            let retry_after = match self.transport.get(request).await {
                Err(message) => {
                    last_error = message;
                    None
                }
                Ok(resp) => match resp.status {
                    200..=299 => {
                        let validators = HttpValidators {
                            etag: resp.header("etag").map(str::to_owned),
                            last_modified: resp.header("last-modified").map(str::to_owned),
                        };
                        return Ok(Fetched::Body {
                            body: resp.body,
                            validators,
                        });
                    }
                    304 => return Ok(Fetched::NotModified),
                    404 | 410 => return Ok(Fetched::NotFound),
                    429 | 500..=599 => {
                        last_error = format!("HTTP {}", resp.status);
                        resp.header("retry-after")
                            .and_then(|s| s.trim().parse::<u64>().ok())
                            .map(Duration::from_secs)
                    }
                    status => {
                        return Err(FetchError::Status {
                            url: url.to_owned(),
                            status,
                        });
                    }
                },
            };
            if attempt + 1 < self.politeness.max_attempts {
                let backoff = self.politeness.retry_base * 2u32.saturating_pow(attempt);
                tokio::time::sleep(retry_after.unwrap_or(backoff).min(MAX_RETRY_WAIT)).await;
            }
        }
        Err(FetchError::Transport {
            url: url.to_owned(),
            message: last_error,
        })
    }

    /// Stores the validators of a processed response for the next run.
    pub fn commit(&self, url: &str, validators: &HttpValidators) -> Result<(), StoreError> {
        self.store.save_http_validators(
            url,
            validators.etag.as_deref(),
            validators.last_modified.as_deref(),
            self.now,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;

    /// `(url, status, headers, body)`
    type Reply<'a> = (&'a str, u16, &'a [(&'a str, &'a str)], &'a str);
    /// `(url, request headers, arrival time)`
    type Logged = (String, Vec<(String, String)>, Instant);

    /// Replies from a script and records when each request arrived.
    #[derive(Debug)]
    struct Scripted {
        replies: StdMutex<Vec<(String, HttpResponse)>>,
        log: StdMutex<Vec<Logged>>,
    }

    impl Scripted {
        fn new(replies: Vec<Reply<'_>>) -> Self {
            Self {
                replies: StdMutex::new(
                    replies
                        .into_iter()
                        .rev()
                        .map(|(url, status, headers, body)| {
                            (
                                url.to_owned(),
                                HttpResponse {
                                    status,
                                    headers: headers
                                        .iter()
                                        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                                        .collect(),
                                    body: body.as_bytes().to_vec(),
                                },
                            )
                        })
                        .collect(),
                ),
                log: StdMutex::new(Vec::new()),
            }
        }
    }

    impl Transport for Scripted {
        fn get(&self, request: HttpRequest) -> TransportFuture<'_> {
            self.log.lock().unwrap().push((
                request.url.clone(),
                request.headers.clone(),
                Instant::now(),
            ));
            let (url, resp) = self
                .replies
                .lock()
                .unwrap()
                .pop()
                .expect("unexpected request");
            assert_eq!(url, request.url);
            Box::pin(async move { Ok(resp) })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn robots_first_then_spacing_and_conditional_get() {
        let store = Store::open_in_memory().unwrap();
        let t = Scripted::new(vec![
            ("https://a.test/robots.txt", 404, &[], ""),
            ("https://a.test/board", 200, &[("etag", "\"v1\"")], "body"),
            ("https://a.test/board", 304, &[], ""),
        ]);
        let f = Fetcher::new(&t, &store, Politeness::default(), 100);
        let Fetched::Body { body, validators } = f.get("https://a.test/board", true).await.unwrap()
        else {
            panic!("expected body");
        };
        assert_eq!(body, b"body");
        f.commit("https://a.test/board", &validators).unwrap();
        assert!(matches!(
            f.get("https://a.test/board", true).await.unwrap(),
            Fetched::NotModified
        ));
        let log = t.log.lock().unwrap();
        assert_eq!(
            log[2].1,
            [("if-none-match".to_owned(), "\"v1\"".to_owned())]
        );
        assert!(log[1].2 - log[0].2 >= Duration::from_secs(1));
        assert!(log[2].2 - log[1].2 >= Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn disallowed_path_is_never_requested() {
        let store = Store::open_in_memory().unwrap();
        let t = Scripted::new(vec![(
            "https://b.test/robots.txt",
            200,
            &[],
            "User-agent: *\nDisallow: /private\n",
        )]);
        let f = Fetcher::new(&t, &store, Politeness::default(), 0);
        assert!(matches!(
            f.get("https://b.test/private/x", false).await.unwrap(),
            Fetched::Disallowed
        ));
        assert_eq!(t.log.lock().unwrap().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_on_429_honoring_retry_after() {
        let store = Store::open_in_memory().unwrap();
        let t = Scripted::new(vec![
            ("https://c.test/robots.txt", 404, &[], ""),
            ("https://c.test/x", 429, &[("retry-after", "5")], ""),
            ("https://c.test/x", 503, &[], ""),
            ("https://c.test/x", 200, &[], "ok"),
        ]);
        let f = Fetcher::new(&t, &store, Politeness::default(), 0);
        assert!(matches!(
            f.get("https://c.test/x", false).await.unwrap(),
            Fetched::Body { .. }
        ));
        let log = t.log.lock().unwrap();
        assert!(log[2].2 - log[1].2 >= Duration::from_secs(5));
        assert!(
            log[3].2 - log[2].2 >= Duration::from_secs(4),
            "2 s base doubled"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn excluded_hosts_get_no_request_at_all() {
        let store = Store::open_in_memory().unwrap();
        let t = Scripted::new(vec![]);
        let f = Fetcher::new(&t, &store, Politeness::default(), 0);
        for url in [
            "https://nvidia.wd5.myworkdayjobs.com/en-US/careers/job/1",
            "https://www.workday.com/",
            "https://app.joinhandshake.com/jobs/1",
            "https://www.linkedin.com/jobs/view/1",
        ] {
            assert!(
                matches!(f.get(url, false).await.unwrap(), Fetched::Disallowed),
                "{url}"
            );
        }
        assert!(t.log.lock().unwrap().is_empty());
        assert!(!is_excluded_host("notworkday.com"));
    }

    #[tokio::test(start_paused = true)]
    async fn unreachable_robots_blocks_host() {
        let store = Store::open_in_memory().unwrap();
        let t = Scripted::new(vec![("https://d.test/robots.txt", 503, &[], "")]);
        let f = Fetcher::new(&t, &store, Politeness::default(), 0);
        assert!(matches!(
            f.get("https://d.test/x", false).await.unwrap(),
            Fetched::Disallowed
        ));
    }
}
