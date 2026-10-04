//! A [`Transport`] that serves saved responses, so discovery runs end to
//! end without network access. Routes come from a TOML manifest:
//!
//! ```toml
//! [[route]]
//! url = "https://boards-api.greenhouse.io/v1/boards/acme/jobs?content=true"
//! file = "greenhouse/acme.json"   # relative to the manifest
//! etag = "\"v1\""                 # optional; enables 304 replies
//! ```
//!
//! Unlisted URLs answer 404, which reads as "no robots.txt" for robots
//! requests and "no such board" for probes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;

use crate::http::{HttpRequest, HttpResponse, Transport, TransportFuture};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(rename = "route")]
    routes: Vec<Route>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    url: String,
    #[serde(default = "ok")]
    status: u16,
    file: Option<String>,
    body: Option<String>,
    etag: Option<String>,
}

const fn ok() -> u16 {
    200
}

#[derive(Debug)]
pub struct FixtureTransport {
    routes: HashMap<String, Route>,
    base: PathBuf,
    requests: Mutex<Vec<String>>,
}

impl FixtureTransport {
    pub fn load(manifest: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(manifest)
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let parsed: Manifest =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
        Ok(Self {
            routes: parsed
                .routes
                .into_iter()
                .map(|r| (r.url.clone(), r))
                .collect(),
            base: manifest.parent().unwrap_or(Path::new(".")).to_path_buf(),
            requests: Mutex::new(Vec::new()),
        })
    }

    /// Every URL requested so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    fn respond(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        let Some(route) = self.routes.get(&request.url) else {
            return Ok(HttpResponse {
                status: 404,
                headers: Vec::new(),
                body: Vec::new(),
            });
        };
        let mut headers = Vec::new();
        if let Some(etag) = &route.etag {
            let sent = request
                .headers
                .iter()
                .any(|(k, v)| k == "if-none-match" && v == etag);
            if sent {
                return Ok(HttpResponse {
                    status: 304,
                    headers: Vec::new(),
                    body: Vec::new(),
                });
            }
            headers.push(("etag".to_owned(), etag.clone()));
        }
        let body = match (&route.file, &route.body) {
            (Some(file), _) => {
                let path = self.base.join(file);
                std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?
            }
            (None, Some(body)) => body.clone().into_bytes(),
            (None, None) => Vec::new(),
        };
        Ok(HttpResponse {
            status: route.status,
            headers,
            body,
        })
    }
}

impl Transport for FixtureTransport {
    fn get(&self, request: HttpRequest) -> TransportFuture<'_> {
        if let Ok(mut log) = self.requests.lock() {
            log.push(request.url.clone());
        }
        let result = self.respond(&request);
        Box::pin(async move { result })
    }
}
