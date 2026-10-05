//! One discovery run: pipeline steps 1–3 of §3 (discover, normalize,
//! filter). Order:
//!
//! 1. register seed, user and manual boards;
//! 2. read list feeds and the HN thread (concurrently; different hosts);
//! 3. probe board slugs for company names collected so far;
//! 4. poll every board, one task per API host;
//! 5. attach feed items to the board postings they point at;
//! 6. evaluate every open posting against the search profile.
//!
//! A failing source is recorded in the report and the run goes on. Only a
//! database error stops the run.

use std::collections::HashSet;
use std::sync::Mutex;

use gantry_core::facts::PostingFacts;
use gantry_core::filter::{FilterContext, Subject, evaluate};
use gantry_core::geo::Place;
use gantry_core::{Ats, Posting, PostingKey};
use gantry_store::{
    CompanyEntry, CompanyRow, ConfigSnapshot, Store, StoreError, UpsertOutcome, UpsertStatus,
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::bundled::{FeedDef, FeedFormat};
use crate::http::{FetchError, Fetched, Fetcher, Politeness, Transport};
use crate::resolver::{Resolved, resolve};
use crate::sources::simplify::FeedItem;
use crate::sources::{ParseError, ashby, greenhouse, hn, job_type_from_text, lever, simplify};
use crate::{geocode, probe};

/// Company names probed per run; the rest wait for the next night. At one
/// request per second per API host this bounds probing to a few minutes.
pub const MAX_PROBE_NAMES_PER_RUN: usize = 200;

/// Errors kept in the report; the count covers all of them.
const MAX_REPORTED_ERRORS: usize = 50;

#[derive(Debug, Clone, Copy)]
pub struct DiscoverInputs<'a> {
    pub snapshot: &'a ConfigSnapshot,
    /// Seed boards for the user's occupations.
    pub seed: &'a [CompanyEntry],
    /// Every bundled feed; the ones `settings.toml` enables are read.
    pub feeds: &'a [FeedDef],
    /// Unix seconds; one clock reading for the whole run.
    pub now: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceOutcome {
    Ok,
    Unchanged,
    NotFound,
    Disallowed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SourceStatus {
    pub source: String,
    pub outcome: SourceOutcome,
    /// Feed listings or HN comments read.
    pub items: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BoardStats {
    pub polled: u32,
    pub unchanged: u32,
    pub not_found: u32,
    pub failed: u32,
}

impl BoardStats {
    fn add(&mut self, o: Self) {
        self.polled += o.polled;
        self.unchanged += o.unchanged;
        self.not_found += o.not_found;
        self.failed += o.failed;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProbeStats {
    pub names: u32,
    pub names_already_known: u32,
    pub boards_found: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PostingStats {
    /// Postings listed by any source this run.
    pub seen: u32,
    pub new: u32,
    pub updated: u32,
    pub reopened: u32,
    pub closed: u32,
    /// New postings linked to an earlier one as reposts.
    pub reposts: u32,
    /// Open postings after the run.
    pub open: u32,
    pub passed: u32,
    pub filtered_out: u32,
    /// Open postings with at least one location that could not be placed.
    pub unresolved_locations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RunError {
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DiscoverReport {
    pub run_id: i64,
    pub started_at: i64,
    pub finished_at: i64,
    pub boards_added: u32,
    pub feeds: Vec<SourceStatus>,
    pub hacker_news: Option<SourceStatus>,
    pub probes: ProbeStats,
    pub boards: BoardStats,
    pub postings: PostingStats,
    /// The geocoded home location the distance filter used.
    pub home: Option<String>,
    pub warnings: Vec<String>,
    pub error_count: u32,
    pub errors: Vec<RunError>,
}

/// A feed item pointing at a board posting, attached after boards are polled.
struct Pending {
    key: PostingKey,
    company_id: i64,
    item: FeedItem,
    source: String,
}

/// One company name on one board API.
enum Probe {
    Hit(String),
    Miss,
    /// A request failed; the name is retried next run.
    Failed,
}

struct Run<'a> {
    store: &'a Store,
    fetcher: Fetcher<'a>,
    inputs: DiscoverInputs<'a>,
    /// Filled in as the run goes; tasks on different hosts share it.
    report: Mutex<DiscoverReport>,
}

pub async fn discover(
    store: &Store,
    transport: &dyn Transport,
    politeness: Politeness,
    inputs: DiscoverInputs<'_>,
) -> Result<DiscoverReport, StoreError> {
    let now = inputs.now;
    let run_id = store.begin_run("discover", now)?;
    let run = Run {
        store,
        fetcher: Fetcher::new(transport, store, politeness, now),
        inputs,
        report: Mutex::new(DiscoverReport {
            run_id,
            started_at: now,
            ..DiscoverReport::default()
        }),
    };
    let settings = &inputs.snapshot.settings.discovery;

    for entry in inputs.seed {
        run.register(entry, "seed")?;
    }
    for entry in &inputs.snapshot.companies {
        run.register(entry, "user")?;
    }
    run.register_manual_urls()?;
    run.retire_removed_boards()?;

    let enabled: Vec<&FeedDef> = inputs
        .feeds
        .iter()
        .filter(|f| settings.feeds.contains(&f.id))
        .collect();
    for id in &settings.feeds {
        if !inputs.feeds.iter().any(|f| &f.id == id) {
            run.tally()
                .warnings
                .push(format!("settings.toml enables unknown feed \"{id}\""));
        }
    }
    let feeds_task = async {
        let mut pending = Vec::new();
        let mut statuses = Vec::new();
        for feed in &enabled {
            statuses.push(run.read_feed(feed, &mut pending).await?);
        }
        Ok::<_, StoreError>((statuses, pending))
    };
    let hn_task = async {
        if settings.hacker_news {
            run.read_hn().await.map(Some)
        } else {
            Ok(None)
        }
    };
    let (feeds_result, hn_result) = tokio::join!(feeds_task, hn_task);
    let (statuses, pending) = feeds_result?;
    let hacker_news = hn_result?;
    run.close_disabled_feeds(&enabled)?;
    {
        let mut report = run.tally();
        report.feeds = statuses;
        report.hacker_news = hacker_news;
    }

    if settings.slug_probing {
        let probes = run.probe().await?;
        run.tally().probes = probes;
    }

    let companies = store.companies()?;
    let group = |pred: fn(Ats) -> bool| -> Vec<CompanyRow> {
        companies.iter().filter(|c| pred(c.ats)).cloned().collect()
    };
    let (gh, lv, lv_eu, ab) = tokio::join!(
        run.poll_group(group(|a| a == Ats::Greenhouse)),
        run.poll_group(group(|a| a == Ats::Lever)),
        run.poll_group(group(|a| a == Ats::LeverEu)),
        run.poll_group(group(|a| a == Ats::Ashby)),
    );
    let mut polled_ok = HashSet::new();
    for result in [gh, lv, lv_eu, ab] {
        let (stats, ok) = result?;
        run.tally().boards.add(stats);
        polled_ok.extend(ok);
    }

    run.attach_pending(pending, &polled_ok)?;
    run.evaluate_all(&mut run.tally())?;

    let mut report = run
        .report
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    report.finished_at = jiff::Timestamp::now().as_second().max(now);
    store.finish_run(
        run_id,
        report.finished_at,
        i64::from(report.postings.seen),
        &serde_json::to_string(&report.errors)?,
        &serde_json::to_string(&report)?,
    )?;
    Ok(report)
}

fn board_url(ats: Ats, token: &str) -> Option<String> {
    match ats {
        Ats::Greenhouse => Some(greenhouse::board_url(token)),
        Ats::Lever | Ats::LeverEu => Some(lever::board_url(ats, token)),
        Ats::Ashby => Some(ashby::board_url(token)),
        Ats::External => None,
    }
}

fn parse_board(c: &CompanyRow, body: &[u8]) -> Result<Vec<Posting>, ParseError> {
    match c.ats {
        Ats::Greenhouse => greenhouse::parse_board(&c.token, &c.name, body),
        Ats::Lever | Ats::LeverEu => lever::parse_board(c.ats, &c.token, &c.name, body),
        Ats::Ashby => ashby::parse_board(&c.token, &c.name, body),
        Ats::External => Ok(Vec::new()),
    }
}

/// A posting built from a feed listing alone.
fn feed_posting(key: PostingKey, item: &FeedItem) -> Posting {
    Posting {
        key,
        company_name: item.company.clone(),
        title: item.title.clone(),
        url: item.url.clone(),
        locations: geocode::locate_all(&item.locations),
        locations_raw: item.locations.clone(),
        work_mode: None,
        job_type: job_type_from_text(&item.title),
        published_at: item.posted_at,
        description: String::new(),
        facts: PostingFacts {
            sponsorship: item.sponsorship.clone(),
            ..PostingFacts::default()
        },
    }
}

impl Run<'_> {
    fn now(&self) -> i64 {
        self.inputs.now
    }

    /// The report under construction. A poisoned lock only means another
    /// task panicked mid-update; the counts are still usable.
    fn tally(&self) -> std::sync::MutexGuard<'_, DiscoverReport> {
        self.report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn error(&self, source: &str, message: impl std::fmt::Display) {
        tracing::warn!(source, %message, "discovery source failed");
        let mut report = self.tally();
        report.error_count += 1;
        if report.errors.len() < MAX_REPORTED_ERRORS {
            report.errors.push(RunError {
                source: source.to_owned(),
                message: message.to_string(),
            });
        }
    }

    fn count(&self, outcome: &UpsertOutcome) {
        let s = &mut self.tally().postings;
        s.seen += 1;
        match outcome.status {
            UpsertStatus::New => s.new += 1,
            UpsertStatus::Updated => s.updated += 1,
            UpsertStatus::Reopened => s.reopened += 1,
            UpsertStatus::Unchanged => {}
        }
        if outcome.repost_of.is_some() {
            s.reposts += 1;
        }
    }

    fn closed(&self, n: u32) {
        self.tally().postings.closed += n;
    }

    fn register(&self, entry: &CompanyEntry, source: &str) -> Result<i64, StoreError> {
        let (id, added) = self.store.upsert_company(entry, source, self.now())?;
        self.tally().boards_added += u32::from(added);
        Ok(id)
    }

    /// Registers the board behind a resolved URL. Returns its company id.
    fn register_resolved(
        &self,
        resolved: &Resolved,
        company: Option<&str>,
        source: &str,
    ) -> Result<Option<i64>, StoreError> {
        match resolved {
            Resolved::Posting {
                ats, board_token, ..
            }
            | Resolved::Board { ats, board_token } => {
                let entry = CompanyEntry {
                    name: company.unwrap_or(board_token).to_owned(),
                    ats: *ats,
                    token: board_token.clone(),
                    staffing_agency: None,
                };
                self.register(&entry, source).map(Some)
            }
            Resolved::GreenhouseEmbed { host, .. } => {
                if let Some(name) = company
                    .map(str::to_owned)
                    .or_else(|| probe::host_company(host))
                {
                    self.store.queue_probe_name(&name, source, self.now())?;
                }
                Ok(None)
            }
            Resolved::External { .. } => Ok(None),
        }
    }

    /// Seed and user boards no longer in either list stop being polled.
    /// Boards found by other sources stay; a feed or HN that still points
    /// at a retired board registers it again.
    fn retire_removed_boards(&self) -> Result<(), StoreError> {
        let listed: HashSet<(Ats, String)> = self
            .inputs
            .seed
            .iter()
            .chain(&self.inputs.snapshot.companies)
            .map(|e| (e.ats, e.ats.canonical_token(&e.token)))
            .collect();
        for c in self.store.companies()? {
            if matches!(c.source.as_str(), "seed" | "user")
                && !listed.contains(&(c.ats, c.token.clone()))
            {
                if let Some(url) = board_url(c.ats, &c.token) {
                    self.fetcher.forget(&url)?;
                }
                let closed = self.store.retire_company(&c, self.now())?;
                self.closed(closed);
            }
        }
        Ok(())
    }

    /// Postings only a feed no longer enabled (or no longer bundled)
    /// listed take misses, as if the feed had dropped them.
    fn close_disabled_feeds(&self, enabled: &[&FeedDef]) -> Result<(), StoreError> {
        let enabled: HashSet<String> = enabled.iter().map(|f| f.source()).collect();
        for source in self.store.posting_source_names()? {
            if source.starts_with("feed:") && !enabled.contains(&source) {
                if let Some(feed) = self.inputs.feeds.iter().find(|f| f.source() == source) {
                    self.fetcher.forget(&feed.url)?;
                }
                let closed =
                    self.store
                        .reconcile_feed(&source, Some(&HashSet::new()), self.now())?;
                self.closed(closed);
            }
        }
        Ok(())
    }

    fn register_manual_urls(&self) -> Result<(), StoreError> {
        for (url, json) in self.store.manual_urls()? {
            match serde_json::from_str::<Resolved>(&json) {
                Ok(resolved) => {
                    self.register_resolved(&resolved, None, "manual")?;
                }
                Err(e) => self.error("manual", format!("{url}: {e}")),
            }
        }
        Ok(())
    }

    /// A polite GET whose non-database failures are recorded and become `None`.
    async fn get(
        &self,
        source: &str,
        url: &str,
        conditional: bool,
    ) -> Result<Option<Fetched>, StoreError> {
        match self.fetcher.get(url, conditional).await {
            Ok(f) => Ok(Some(f)),
            Err(FetchError::Store(e)) => Err(e),
            Err(e) => {
                self.error(source, e);
                Ok(None)
            }
        }
    }

    async fn read_feed(
        &self,
        feed: &FeedDef,
        pending: &mut Vec<Pending>,
    ) -> Result<SourceStatus, StoreError> {
        let source = feed.source();
        let status = |outcome, items| SourceStatus {
            source: source.clone(),
            outcome,
            items,
        };
        let (body, validators) = match self.get(&source, &feed.url, true).await? {
            None => return Ok(status(SourceOutcome::Failed, 0)),
            Some(Fetched::NotModified) => {
                let closed = self.store.reconcile_feed(&source, None, self.now())?;
                self.closed(closed);
                return Ok(status(SourceOutcome::Unchanged, 0));
            }
            Some(Fetched::NotFound) => {
                self.error(&source, format!("{} not found", feed.url));
                return Ok(status(SourceOutcome::NotFound, 0));
            }
            Some(Fetched::Disallowed) => {
                self.error(&source, "disallowed by robots.txt");
                return Ok(status(SourceOutcome::Disallowed, 0));
            }
            Some(Fetched::Body { body, validators }) => (body, validators),
        };
        let items = match feed.format {
            FeedFormat::SimplifyListings => simplify::parse(&body),
        };
        let items = match items {
            Ok(items) => items,
            Err(e) => {
                self.error(&source, e);
                return Ok(status(SourceOutcome::Failed, 0));
            }
        };
        let mut seen = HashSet::new();
        for item in &items {
            self.store
                .queue_probe_name(&item.company, &source, self.now())?;
            let resolved = resolve(&item.url).unwrap_or(Resolved::External {
                url: item.url.clone(),
            });
            let company_id = self.register_resolved(&resolved, Some(&item.company), &source)?;
            if let (
                Resolved::Posting {
                    ats,
                    board_token,
                    job_id,
                },
                Some(company_id),
            ) = (&resolved, company_id)
            {
                let key = PostingKey {
                    ats: *ats,
                    board_token: board_token.clone(),
                    job_id: job_id.clone(),
                };
                seen.insert(key.clone());
                pending.push(Pending {
                    key,
                    company_id,
                    item: item.clone(),
                    source: source.clone(),
                });
                continue;
            }
            let key = PostingKey {
                ats: Ats::External,
                board_token: source.clone(),
                job_id: item.id.clone(),
            };
            let outcome = self.store.upsert_posting(
                &feed_posting(key.clone(), item),
                None,
                &source,
                &item.url,
                self.now(),
            )?;
            self.count(&outcome);
            seen.insert(key);
        }
        let closed = self
            .store
            .reconcile_feed(&source, Some(&seen), self.now())?;
        self.closed(closed);
        self.fetcher.commit(&feed.url, &validators)?;
        Ok(status(
            SourceOutcome::Ok,
            u32::try_from(items.len()).unwrap_or(u32::MAX),
        ))
    }

    async fn read_hn(&self) -> Result<SourceStatus, StoreError> {
        const SOURCE: &str = "hn";
        let status = |outcome, items| SourceStatus {
            source: SOURCE.into(),
            outcome,
            items,
        };
        let Some(Fetched::Body { body, .. }) = self.get(SOURCE, hn::USER_URL, false).await? else {
            return Ok(status(SourceOutcome::Failed, 0));
        };
        let submitted = match hn::parse_user(&body) {
            Ok(ids) => ids,
            Err(e) => {
                self.error(SOURCE, e);
                return Ok(status(SourceOutcome::Failed, 0));
            }
        };
        let mut thread = None;
        for id in submitted.iter().take(hn::THREAD_SEARCH_DEPTH) {
            if let Some(item) = self.hn_item(*id).await?
                && hn::is_hiring_thread(&item)
            {
                thread = Some(item);
                break;
            }
        }
        let Some(thread) = thread else {
            self.error(
                SOURCE,
                "no \"Who is hiring?\" thread among recent submissions",
            );
            return Ok(status(SourceOutcome::NotFound, 0));
        };
        let mut read = 0;
        for kid in &thread.kids {
            let item_id = kid.to_string();
            if self.store.is_processed(SOURCE, &item_id)? {
                continue;
            }
            // A failed request is retried next run; an item the API answers
            // 404 or `null` for (deleted) is done.
            let item = match self.get(SOURCE, &hn::item_url(*kid), false).await? {
                Some(Fetched::Body { body, .. }) => match hn::parse_item(&body) {
                    Ok(item) => item,
                    Err(e) => {
                        self.error(SOURCE, e);
                        continue;
                    }
                },
                Some(Fetched::NotFound) => None,
                _ => continue,
            };
            if let Some(lead) = item.as_ref().and_then(hn::parse_comment) {
                if let Some(company) = &lead.company {
                    self.store.queue_probe_name(company, SOURCE, self.now())?;
                }
                for url in &lead.urls {
                    if let Ok(resolved) = resolve(url) {
                        self.register_resolved(&resolved, lead.company.as_deref(), SOURCE)?;
                    }
                }
            }
            self.store.mark_processed(SOURCE, &item_id, self.now())?;
            read += 1;
        }
        Ok(status(SourceOutcome::Ok, read))
    }

    /// One HN item; `None` when missing or on a recorded failure.
    async fn hn_item(&self, id: u64) -> Result<Option<hn::Item>, StoreError> {
        let Some(Fetched::Body { body, .. }) = self.get("hn", &hn::item_url(id), false).await?
        else {
            return Ok(None);
        };
        match hn::parse_item(&body) {
            Ok(item) => Ok(item),
            Err(e) => {
                self.error("hn", e);
                Ok(None)
            }
        }
    }

    async fn probe(&self) -> Result<ProbeStats, StoreError> {
        let mut stats = ProbeStats::default();
        let known: HashSet<String> = self
            .store
            .companies()?
            .iter()
            .map(|c| probe::slug_variants(&c.name).concat())
            .collect();
        for name in self
            .store
            .probe_batch(self.now(), MAX_PROBE_NAMES_PER_RUN)?
        {
            stats.names += 1;
            let variants = probe::slug_variants(&name);
            if variants.is_empty() || known.contains(&variants.concat()) {
                stats.names_already_known += u32::from(!variants.is_empty());
                self.store
                    .finish_probe(&name, !variants.is_empty(), self.now())?;
                continue;
            }
            let (gh, lv, ab) = tokio::join!(
                self.probe_ats(Ats::Greenhouse, &variants, &name),
                self.probe_ats(Ats::Lever, &variants, &name),
                self.probe_ats(Ats::Ashby, &variants, &name),
            );
            let mut found = false;
            let mut complete = true;
            for (ats, result) in [(Ats::Greenhouse, gh?), (Ats::Lever, lv?), (Ats::Ashby, ab?)] {
                match result {
                    Probe::Hit(slug) => {
                        let entry = CompanyEntry {
                            name: name.clone(),
                            ats,
                            token: slug,
                            staffing_agency: None,
                        };
                        self.register(&entry, "probe")?;
                        stats.boards_found += 1;
                        found = true;
                    }
                    Probe::Miss => {}
                    Probe::Failed => complete = false,
                }
            }
            // A probe that failed on the network is retried next run.
            if complete || found {
                self.store.finish_probe(&name, found, self.now())?;
            }
        }
        Ok(stats)
    }

    async fn probe_ats(
        &self,
        ats: Ats,
        variants: &[String],
        name: &str,
    ) -> Result<Probe, StoreError> {
        for slug in variants {
            match self.store.cached_probe(slug, ats, self.now())? {
                Some(true) => return Ok(Probe::Hit(slug.clone())),
                Some(false) => continue,
                None => {}
            }
            let url = match ats {
                Ats::Greenhouse => greenhouse::probe_url(slug),
                Ats::Lever | Ats::LeverEu => lever::probe_url(slug),
                Ats::Ashby => ashby::probe_url(slug),
                Ats::External => return Ok(Probe::Miss),
            };
            let hit = match self.get(&format!("probe:{ats}"), &url, false).await? {
                // Not an answer about the slug, so nothing is cached.
                None | Some(Fetched::Disallowed) => return Ok(Probe::Failed),
                // A slug shared by an unrelated company is rejected:
                // Greenhouse names the board; Lever and Ashby do not, so
                // their postings must mention the company.
                Some(Fetched::Body { body, .. }) => match ats {
                    Ats::Greenhouse => greenhouse::parse_board_name(&body)
                        .is_some_and(|board| probe::names_match(&board, name)),
                    Ats::Lever | Ats::LeverEu => lever::parse_board(ats, slug, name, &body)
                        .is_ok_and(|ps| {
                            probe::mentions_company(ps.iter().map(|p| p.description.as_str()), name)
                        }),
                    Ats::Ashby => ashby::parse_board(slug, name, &body).is_ok_and(|ps| {
                        probe::mentions_company(ps.iter().map(|p| p.description.as_str()), name)
                    }),
                    Ats::External => false,
                },
                Some(Fetched::NotFound | Fetched::NotModified) => false,
            };
            self.store.record_probe(slug, ats, hit, self.now())?;
            if hit {
                return Ok(Probe::Hit(slug.clone()));
            }
        }
        Ok(Probe::Miss)
    }

    async fn poll_group(
        &self,
        companies: Vec<CompanyRow>,
    ) -> Result<(BoardStats, HashSet<(Ats, String)>), StoreError> {
        let mut stats = BoardStats::default();
        let mut ok = HashSet::new();
        for c in companies {
            let Some(url) = board_url(c.ats, &c.token) else {
                continue;
            };
            let source = c.ats.as_str();
            let fail = |stats: &mut BoardStats, message: &str| -> Result<(), StoreError> {
                stats.failed += 1;
                self.store.record_poll(c.id, self.now(), Some(message))
            };
            match self.get(source, &url, true).await? {
                None => fail(&mut stats, "request failed")?,
                Some(Fetched::Disallowed) => {
                    self.error(source, format!("{url}: disallowed by robots.txt"));
                    fail(&mut stats, "disallowed by robots.txt")?;
                }
                Some(Fetched::NotFound) => {
                    // A deleted board lists nothing; two 404s in a row
                    // close its postings like any other two misses.
                    stats.not_found += 1;
                    let closed = self.store.reconcile_board(
                        c.ats,
                        &c.token,
                        Some(&HashSet::new()),
                        self.now(),
                    )?;
                    self.closed(closed);
                    self.fetcher.forget(&url)?;
                    self.store
                        .record_poll(c.id, self.now(), Some("board not found"))?;
                    ok.insert((c.ats, c.token.clone()));
                }
                Some(Fetched::NotModified) => {
                    stats.unchanged += 1;
                    let closed = self
                        .store
                        .reconcile_board(c.ats, &c.token, None, self.now())?;
                    self.closed(closed);
                    self.store.record_poll(c.id, self.now(), None)?;
                    ok.insert((c.ats, c.token.clone()));
                }
                Some(Fetched::Body { body, validators }) => match parse_board(&c, &body) {
                    Err(e) => {
                        self.error(source, format!("{}: {e}", c.token));
                        fail(&mut stats, "unexpected response")?;
                    }
                    Ok(postings) => {
                        let mut seen = HashSet::new();
                        for p in postings {
                            let outcome = self.store.upsert_posting(
                                &p,
                                Some(c.id),
                                source,
                                &p.url,
                                self.now(),
                            )?;
                            self.count(&outcome);
                            seen.insert(p.key.job_id);
                        }
                        let closed =
                            self.store
                                .reconcile_board(c.ats, &c.token, Some(&seen), self.now())?;
                        self.closed(closed);
                        self.fetcher.commit(&url, &validators)?;
                        self.store.record_poll(c.id, self.now(), None)?;
                        stats.polled += 1;
                        ok.insert((c.ats, c.token.clone()));
                    }
                },
            }
        }
        Ok((stats, ok))
    }

    /// Feed items that point at a board posting: record the feed as a
    /// source of that posting, with its sponsorship statement. If the board
    /// was polled (or is gone) and did not list the job, the stale feed
    /// entry is ignored. If the board could not be polled, the feed's data
    /// stands in.
    fn attach_pending(
        &self,
        pending: Vec<Pending>,
        polled_ok: &HashSet<(Ats, String)>,
    ) -> Result<(), StoreError> {
        for p in pending {
            let polled = polled_ok.contains(&(p.key.ats, p.key.board_token.clone()));
            if let Some(id) =
                self.store
                    .add_source_if_exists(&p.key, &p.source, &p.item.url, self.now())?
            {
                self.store
                    .set_feed_sponsorship(id, p.item.sponsorship.as_ref())?;
                // The board could not say; the feed's listing keeps the
                // posting alive (and reopens a stand-in it had closed).
                if !polled && self.store.mark_seen(id, self.now())? {
                    self.tally().postings.reopened += 1;
                }
                continue;
            }
            if polled {
                continue;
            }
            let posting = feed_posting(p.key, &p.item);
            let outcome = self.store.upsert_posting(
                &posting,
                Some(p.company_id),
                &p.source,
                &p.item.url,
                self.now(),
            )?;
            self.count(&outcome);
        }
        Ok(())
    }

    fn evaluate_all(&self, report: &mut DiscoverReport) -> Result<(), StoreError> {
        let search = &self.inputs.snapshot.search;
        let gaz = geocode::gazetteer();
        let home = match search.location.home.as_deref() {
            Some(text) => {
                let place = gaz.locate(text);
                if place.is_none() {
                    report.warnings.push(format!(
                        "home location \"{text}\" was not found in the bundled place data; \
                         distance filters cannot run"
                    ));
                }
                place
            }
            None => None,
        };
        report.home = home.as_ref().map(describe_place);
        let relocate: Vec<Place> = search
            .location
            .relocate
            .places
            .iter()
            .filter_map(|text| {
                let place = gaz.locate(text);
                if place.is_none() {
                    report
                        .warnings
                        .push(format!("relocation place \"{text}\" was not found"));
                }
                place
            })
            .collect();
        let ctx = FilterContext {
            search,
            banks: &self.inputs.snapshot.banks,
            home: home.as_ref(),
            relocate_places: &relocate,
            now: self.now(),
        };
        for sp in self.store.open_postings()? {
            // Locations and pattern facts are derived here, not stored at
            // ingest, so a better gazetteer or detector applies to every
            // open posting on the next run, unchanged boards included.
            let mut posting = sp.posting;
            posting.locations = geocode::locate_all(&posting.locations_raw);
            posting.facts.fill_from_text(&posting.description);
            let outcome = evaluate(
                &Subject {
                    posting: &posting,
                    first_seen: sp.first_seen,
                    staffing_agency: sp.staffing_agency,
                },
                &ctx,
            );
            self.store.save_evaluation(
                sp.id,
                &outcome,
                &serde_json::to_string(&posting.facts)?,
                self.now(),
            )?;
            let s = &mut report.postings;
            s.open += 1;
            if outcome.passed {
                s.passed += 1;
            } else {
                s.filtered_out += 1;
            }
            if posting.locations.iter().any(|l| l.unresolved.is_some()) {
                s.unresolved_locations += 1;
            }
        }
        Ok(())
    }
}

fn describe_place(p: &Place) -> String {
    match &p.admin1 {
        Some(a) if !a.chars().all(|c| c.is_ascii_digit()) => {
            format!("{}, {a}, {}", p.name, p.country)
        }
        _ => format!("{}, {}", p.name, p.country),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use gantry_store::CompanyEntry;

    use super::*;
    use crate::http::{HttpRequest, HttpResponse, TransportFuture};

    const BOARD: &str =
        "https://boards-api.greenhouse.io/v1/boards/pinecrestrobotics/jobs?content=true";
    const FEED: &str = "https://raw.githubusercontent.com/test/feed.json";
    const BOARD_BODY: &[u8] =
        include_bytes!("../../../fixtures/http/greenhouse/pinecrestrobotics.json");
    /// One listing for a Pinecrest job the board itself does not list.
    const FEED_BODY: &str = r#"[{"id": "s1", "company_name": "Pinecrest Robotics",
        "title": "Robotics Intern", "locations": ["Seattle, WA"], "active": true,
        "url": "https://job-boards.greenhouse.io/pinecrestrobotics/jobs/9999"}]"#;

    /// Answers each URL with its configured status. A board 200 carries an
    /// ETag and becomes a 304 when that ETag comes back; the feed sends
    /// none, like a list that changes every run.
    #[derive(Debug, Default)]
    struct Server {
        status: Mutex<HashMap<String, u16>>,
    }

    impl Server {
        fn set(&self, url: &str, status: u16) {
            self.status.lock().unwrap().insert(url.to_owned(), status);
        }
    }

    impl Transport for Server {
        fn get(&self, request: HttpRequest) -> TransportFuture<'_> {
            let status = if request.url.ends_with("/robots.txt") {
                404
            } else {
                self.status.lock().unwrap()[&request.url]
            };
            let is_feed = request.url == FEED;
            let body = if is_feed {
                FEED_BODY.as_bytes()
            } else {
                BOARD_BODY
            };
            let etag = format!("\"{}\"", body.len());
            let revalidated = request
                .headers
                .iter()
                .any(|(k, v)| k == "if-none-match" && *v == etag);
            Box::pin(async move {
                Ok(match status {
                    200 if revalidated => HttpResponse {
                        status: 304,
                        headers: vec![],
                        body: vec![],
                    },
                    200 => HttpResponse {
                        status,
                        headers: if is_feed {
                            vec![]
                        } else {
                            vec![("etag".into(), etag)]
                        },
                        body: body.to_vec(),
                    },
                    _ => HttpResponse {
                        status,
                        headers: vec![],
                        body: vec![],
                    },
                })
            })
        }
    }

    fn seed() -> Vec<CompanyEntry> {
        vec![CompanyEntry {
            name: "Pinecrest Robotics".into(),
            ats: Ats::Greenhouse,
            token: "pinecrestrobotics".into(),
            staffing_agency: None,
        }]
    }

    fn feed() -> FeedDef {
        FeedDef {
            id: "test".into(),
            name: "Test feed".into(),
            format: FeedFormat::SimplifyListings,
            url: FEED.into(),
            homepage: None,
            occupations: vec![],
        }
    }

    async fn run_with(
        store: &Store,
        server: &Server,
        seed: &[CompanyEntry],
        feed_enabled: bool,
        now: i64,
    ) -> Vec<String> {
        let mut snapshot = ConfigSnapshot::default();
        snapshot.settings.discovery.slug_probing = false;
        if feed_enabled {
            snapshot.settings.discovery.feeds = vec!["test".into()];
        }
        let fast = Politeness {
            min_interval: Duration::ZERO,
            retry_base: Duration::ZERO,
            max_attempts: 1,
        };
        let feeds = [feed()];
        let inputs = DiscoverInputs {
            snapshot: &snapshot,
            seed,
            feeds: &feeds,
            now,
        };
        discover(store, server, fast, inputs).await.unwrap();
        store
            .open_postings()
            .unwrap()
            .into_iter()
            .map(|p| p.posting.key.job_id)
            .collect()
    }

    async fn run(store: &Store, server: &Server, seed: &[CompanyEntry], now: i64) -> usize {
        run_with(store, server, seed, false, now).await.len()
    }

    #[tokio::test]
    async fn board_back_unchanged_after_a_404_keeps_its_postings() {
        let store = Store::open_in_memory().unwrap();
        let server = Server::default();
        server.set(BOARD, 200);
        let open = run(&store, &server, &seed(), 1).await;
        assert!(open > 0);
        server.set(BOARD, 404);
        run(&store, &server, &seed(), 2).await;
        server.set(BOARD, 200);
        assert_eq!(run(&store, &server, &seed(), 3).await, open);
    }

    #[tokio::test]
    async fn retired_board_added_back_reopens_its_postings() {
        let store = Store::open_in_memory().unwrap();
        let server = Server::default();
        server.set(BOARD, 200);
        let open = run(&store, &server, &seed(), 1).await;
        assert_eq!(run(&store, &server, &[], 2).await, 0, "retired");
        assert_eq!(run(&store, &server, &seed(), 3).await, open);
    }

    #[tokio::test]
    async fn stand_in_closes_once_its_board_polls_without_it() {
        let store = Store::open_in_memory().unwrap();
        let server = Server::default();
        server.set(FEED, 200);
        server.set(BOARD, 500);
        let open = run_with(&store, &server, &seed(), true, 1).await;
        assert!(
            open.contains(&"9999".to_owned()),
            "stand-in while board is down"
        );
        server.set(BOARD, 200);
        for now in [2, 3] {
            run_with(&store, &server, &seed(), true, now).await;
        }
        let open = run_with(&store, &server, &seed(), true, 4).await;
        assert!(
            !open.contains(&"9999".to_owned()),
            "board polled twice without it"
        );
    }

    #[tokio::test]
    async fn stand_in_reopens_when_its_feed_lists_it_again() {
        let store = Store::open_in_memory().unwrap();
        let server = Server::default();
        server.set(FEED, 200);
        server.set(BOARD, 500);
        run_with(&store, &server, &seed(), true, 1).await;
        run_with(&store, &server, &seed(), false, 2).await;
        let open = run_with(&store, &server, &seed(), false, 3).await;
        assert!(!open.contains(&"9999".to_owned()), "feed disabled twice");
        let open = run_with(&store, &server, &seed(), true, 4).await;
        assert!(
            open.contains(&"9999".to_owned()),
            "listed again, board still down"
        );
    }
}
