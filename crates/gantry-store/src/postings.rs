//! Posting persistence: upsert by primary key, repost linking, and closed
//! detection (§4.2).

use std::collections::HashSet;

use gantry_core::facts::{Fact, FactSource, Sponsorship};
use gantry_core::filter::{Check, FilterOutcome};
use gantry_core::text::{REPOST_SIMHASH_BITS, fnv1a64, hamming, normalize, simhash};
use gantry_core::{Ats, Posting, PostingKey, WorkMode};
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::{ats_at, log_event};
use crate::{Store, StoreError};

/// A board must miss a posting on this many consecutive successful polls
/// before it counts as closed; one miss is often a transient API glitch.
const MISSES_TO_CLOSE: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertStatus {
    New,
    /// Title, location or description changed.
    Updated,
    Unchanged,
    /// Was closed and is listed again.
    Reopened,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertOutcome {
    pub id: i64,
    pub status: UpsertStatus,
    /// Set when a new posting was linked to an earlier one as a repost.
    pub repost_of: Option<i64>,
}

/// An open posting with what filters need from the store.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPosting {
    pub id: i64,
    pub posting: Posting,
    pub first_seen: i64,
    pub staffing_agency: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilter {
    /// `Some(true)`: passed filters only; `Some(false)`: filtered out only.
    pub passed: Option<bool>,
    pub include_closed: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostingRow {
    pub id: i64,
    pub ats: Ats,
    pub company_name: String,
    pub title: String,
    pub url: String,
    pub location_raw: String,
    pub published_at: Option<i64>,
    pub first_seen: i64,
    pub closed_at: Option<i64>,
    pub duplicate_of: Option<i64>,
    /// `None` until the posting has been evaluated.
    pub passed: Option<bool>,
    pub checks: Vec<Check>,
    pub sources: Vec<String>,
}

fn work_mode_str(mode: WorkMode) -> &'static str {
    match mode {
        WorkMode::Remote => "remote",
        WorkMode::Hybrid => "hybrid",
        WorkMode::Onsite => "onsite",
    }
}

fn add_source(
    conn: &Connection,
    posting_id: i64,
    source: &str,
    source_url: &str,
    now: i64,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO posting_sources (posting_id, source, source_url, first_seen, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT (posting_id, source) DO UPDATE SET source_url = ?3, seen_at = ?4",
        params![posting_id, source, source_url, now],
    )?;
    Ok(())
}

fn find_id(conn: &Connection, key: &PostingKey) -> Result<Option<i64>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT id FROM postings WHERE ats = ?1 AND board_token = ?2 AND ats_job_id = ?3",
            params![key.ats.as_str(), key.board_token, key.job_id],
            |r| r.get(0),
        )
        .optional()?)
}

impl Store {
    /// Inserts or refreshes a posting and records `source` as having seen it.
    pub fn upsert_posting(
        &self,
        posting: &Posting,
        company_id: Option<i64>,
        source: &str,
        source_url: &str,
        now: i64,
    ) -> Result<UpsertOutcome, StoreError> {
        let title_norm = normalize(&posting.title);
        let company_norm = normalize(&posting.company_name);
        let location_raw = posting.locations_raw.join("; ");
        let location_norm = normalize(&location_raw);
        let content_hash = format!(
            "{:016x}",
            fnv1a64(
                format!("{}\n{location_raw}\n{}", posting.title, posting.description).as_bytes()
            )
        );
        // Stored as the i64 with the same bits; SQLite has no unsigned type.
        let sim = simhash(&posting.description) as i64;
        let point = posting
            .locations
            .iter()
            .find_map(|l| l.place.as_ref())
            .map(|p| (p.lat, p.lon));
        let remote_mode = posting.work_mode.map(work_mode_str);
        let key = &posting.key;

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let existing: Option<(i64, String, Option<i64>, String)> = tx
            .query_row(
                "SELECT id, content_hash, closed_at, posting_json FROM postings
                 WHERE ats = ?1 AND board_token = ?2 AND ats_job_id = ?3",
                params![key.ats.as_str(), key.board_token, key.job_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        // A board refresh does not know what a feed said about sponsorship;
        // keep the feed's fact until a structured source says otherwise.
        let mut posting = std::borrow::Cow::Borrowed(posting);
        if posting.facts.sponsorship.is_none()
            && let Some((.., old_json)) = &existing
            && let Some(kept) = feed_sponsorship(old_json)?
        {
            posting.to_mut().facts.sponsorship = Some(kept);
        }
        let json = serde_json::to_string(&*posting)?;

        let outcome = match existing {
            None => {
                tx.execute(
                    "INSERT INTO postings (company_id, ats, board_token, ats_job_id, company_name,
                        company_norm, url, title, title_norm, location_raw, location_norm, lat,
                        lon, remote_mode, posting_json, content_hash, simhash, published_at,
                        first_seen, last_seen)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                        ?16, ?17, ?18, ?19, ?19)",
                    params![
                        company_id,
                        key.ats.as_str(),
                        key.board_token,
                        key.job_id,
                        posting.company_name,
                        company_norm,
                        posting.url,
                        posting.title,
                        title_norm,
                        location_raw,
                        location_norm,
                        point.map(|p| p.0),
                        point.map(|p| p.1),
                        remote_mode,
                        json,
                        content_hash,
                        sim,
                        posting.published_at,
                        now
                    ],
                )?;
                let id = tx.last_insert_rowid();
                log_event(&tx, "posting", id, None, "discovered", now)?;
                // Feed-only postings have no description; every SimHash of
                // empty text is equal, so there is nothing to compare.
                let repost_of = if posting.description.trim().is_empty() {
                    None
                } else {
                    link_repost(&tx, id, &company_norm, &title_norm, &location_norm, sim)?
                };
                UpsertOutcome {
                    id,
                    status: UpsertStatus::New,
                    repost_of,
                }
            }
            Some((id, old_hash, closed_at, _)) => {
                let status = if closed_at.is_some() {
                    log_event(&tx, "posting", id, Some("closed"), "open", now)?;
                    UpsertStatus::Reopened
                } else if old_hash != content_hash {
                    UpsertStatus::Updated
                } else {
                    UpsertStatus::Unchanged
                };
                tx.execute(
                    "UPDATE postings SET company_id = COALESCE(?2, company_id), company_name = ?3,
                        company_norm = ?4, url = ?5, title = ?6, title_norm = ?7,
                        location_raw = ?8, location_norm = ?9, lat = ?10, lon = ?11,
                        remote_mode = ?12, posting_json = ?13, content_hash = ?14, simhash = ?15,
                        published_at = ?16, last_seen = ?17, missed_polls = 0, closed_at = NULL
                     WHERE id = ?1",
                    params![
                        id,
                        company_id,
                        posting.company_name,
                        company_norm,
                        posting.url,
                        posting.title,
                        title_norm,
                        location_raw,
                        location_norm,
                        point.map(|p| p.0),
                        point.map(|p| p.1),
                        remote_mode,
                        json,
                        content_hash,
                        sim,
                        posting.published_at,
                        now
                    ],
                )?;
                UpsertOutcome {
                    id,
                    status,
                    repost_of: None,
                }
            }
        };
        add_source(&tx, outcome.id, source, source_url, now)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Records that `source` also lists an already-stored posting. Returns
    /// `None` if no posting has that key.
    pub fn add_source_if_exists(
        &self,
        key: &PostingKey,
        source: &str,
        source_url: &str,
        now: i64,
    ) -> Result<Option<i64>, StoreError> {
        let conn = self.conn()?;
        let Some(id) = find_id(&conn, key)? else {
            return Ok(None);
        };
        add_source(&conn, id, source, source_url, now)?;
        Ok(Some(id))
    }

    /// After a successful poll of a board: every open posting of that board
    /// not in `seen_job_ids` takes a miss, and closes on its second. `None`
    /// is a `304 Not Modified` (see [`settle`]). Returns how many closed.
    pub fn reconcile_board(
        &self,
        ats: Ats,
        board_token: &str,
        seen_job_ids: Option<&HashSet<String>>,
        now: i64,
    ) -> Result<u32, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let open: Vec<(i64, i64, bool)> = {
            let mut stmt = tx.prepare(
                "SELECT id, missed_polls, ats_job_id FROM postings
                 WHERE ats = ?1 AND board_token = ?2 AND closed_at IS NULL",
            )?;
            let rows = stmt.query_map(params![ats.as_str(), board_token], |r| {
                let job_id: String = r.get(2)?;
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    seen_job_ids.is_some_and(|s| s.contains(&job_id)),
                ))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        let closed = settle(&tx, open, seen_job_ids.is_none(), now)?;
        tx.commit()?;
        Ok(closed)
    }

    /// Like [`Store::reconcile_board`] for a list feed: covers the open
    /// postings no source but this feed lists, including ones that stand in
    /// for a board posting whose board could not be polled.
    pub fn reconcile_feed(
        &self,
        source: &str,
        seen: Option<&HashSet<PostingKey>>,
        now: i64,
    ) -> Result<u32, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let open: Vec<(i64, i64, bool)> = {
            let mut stmt = tx.prepare(
                "SELECT p.id, p.missed_polls, p.ats, p.board_token, p.ats_job_id
                 FROM postings p JOIN posting_sources s ON s.posting_id = p.id AND s.source = ?1
                 WHERE p.closed_at IS NULL AND NOT EXISTS (
                     SELECT 1 FROM posting_sources o WHERE o.posting_id = p.id AND o.source != ?1)",
            )?;
            let rows = stmt.query_map([source], |r| {
                let key = PostingKey {
                    ats: ats_at(r, 2)?,
                    board_token: r.get(3)?,
                    job_id: r.get(4)?,
                };
                Ok((r.get(0)?, r.get(1)?, seen.is_some_and(|s| s.contains(&key))))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        let closed = settle(&tx, open, seen.is_none(), now)?;
        tx.commit()?;
        Ok(closed)
    }

    /// Every source that has listed a posting, e.g. `feed:<id>`.
    pub fn posting_source_names(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT DISTINCT source FROM posting_sources ORDER BY source")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Records a feed's sponsorship statement on a posting another source
    /// owns, unless a structured ATS field already set one.
    pub fn set_feed_sponsorship(
        &self,
        posting_id: i64,
        fact: &Fact<Sponsorship>,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let json: String = conn.query_row(
            "SELECT posting_json FROM postings WHERE id = ?1",
            [posting_id],
            |r| r.get(0),
        )?;
        let mut posting: Posting = serde_json::from_str(&json)?;
        if posting
            .facts
            .sponsorship
            .as_ref()
            .is_some_and(|f| f.source != FactSource::Feed)
        {
            return Ok(());
        }
        posting.facts.sponsorship = Some(fact.clone());
        conn.execute(
            "UPDATE postings SET posting_json = ?2 WHERE id = ?1",
            params![posting_id, serde_json::to_string(&posting)?],
        )?;
        Ok(())
    }

    pub fn open_postings(&self) -> Result<Vec<StoredPosting>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT p.id, p.posting_json, p.first_seen, c.staffing_agency
             FROM postings p LEFT JOIN companies c ON c.id = p.company_id
             WHERE p.closed_at IS NULL ORDER BY p.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, json, first_seen, staffing_agency) = row?;
            out.push(StoredPosting {
                id,
                posting: serde_json::from_str(&json)?,
                first_seen,
                staffing_agency,
            });
        }
        Ok(out)
    }

    pub fn save_evaluation(
        &self,
        posting_id: i64,
        outcome: &FilterOutcome,
        facts_json: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        self.conn()?.execute(
            "INSERT INTO evaluations (posting_id, filter_result, filter_reasons_json, facts_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (posting_id) DO UPDATE SET filter_result = ?2, filter_reasons_json = ?3,
                facts_json = ?4, created_at = ?5",
            params![
                posting_id,
                if outcome.passed { "pass" } else { "fail" },
                serde_json::to_string(&outcome.checks)?,
                facts_json,
                now
            ],
        )?;
        Ok(())
    }

    pub fn list_postings(&self, filter: &ListFilter) -> Result<Vec<PostingRow>, StoreError> {
        let conn = self.conn()?;
        let mut sql = String::from(
            "SELECT p.id, p.ats, p.company_name, p.title, p.url, p.location_raw, p.published_at,
                    p.first_seen, p.closed_at, p.duplicate_of, e.filter_result,
                    e.filter_reasons_json,
                    (SELECT group_concat(source, char(31)) FROM
                        (SELECT source FROM posting_sources WHERE posting_id = p.id ORDER BY source))
             FROM postings p LEFT JOIN evaluations e ON e.posting_id = p.id WHERE 1 = 1",
        );
        if !filter.include_closed {
            sql.push_str(" AND p.closed_at IS NULL");
        }
        match filter.passed {
            Some(true) => sql.push_str(" AND e.filter_result = 'pass'"),
            Some(false) => sql.push_str(" AND e.filter_result = 'fail'"),
            None => {}
        }
        sql.push_str(" ORDER BY COALESCE(p.published_at, p.first_seen) DESC, p.id DESC");
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            let row = PostingRow {
                id: r.get(0)?,
                ats: ats_at(r, 1)?,
                company_name: r.get(2)?,
                title: r.get(3)?,
                url: r.get(4)?,
                location_raw: r.get(5)?,
                published_at: r.get(6)?,
                first_seen: r.get(7)?,
                closed_at: r.get(8)?,
                duplicate_of: r.get(9)?,
                passed: r.get::<_, Option<String>>(10)?.map(|v| v == "pass"),
                checks: Vec::new(),
                sources: r
                    .get::<_, Option<String>>(12)?
                    .map(|s| s.split('\u{1f}').map(str::to_owned).collect())
                    .unwrap_or_default(),
            };
            Ok((row, r.get::<_, Option<String>>(11)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (mut row, checks) = row?;
            if let Some(json) = checks {
                row.checks = serde_json::from_str(&json)?;
            }
            out.push(row);
        }
        Ok(out)
    }
}

/// Applies one poll to open postings `(id, missed_polls, listed)`. A
/// listed posting is seen (feed stand-ins are not re-upserted, so the miss
/// count is reset here); an unlisted one takes a miss. With `unchanged` (a
/// 304), the body is the one that was last processed: postings it listed
/// are seen again, and postings it had already missed are still missing.
fn settle(
    conn: &Connection,
    open: Vec<(i64, i64, bool)>,
    unchanged: bool,
    now: i64,
) -> Result<u32, StoreError> {
    let mut closed = 0;
    for (id, missed, listed) in open {
        if listed || (unchanged && missed == 0) {
            conn.execute(
                "UPDATE postings SET last_seen = ?2, missed_polls = 0 WHERE id = ?1",
                params![id, now],
            )?;
        } else {
            closed += u32::from(take_miss(conn, id, missed, now)?);
        }
    }
    Ok(closed)
}

/// A feed-sourced sponsorship fact in a stored posting's JSON.
fn feed_sponsorship(posting_json: &str) -> Result<Option<Fact<Sponsorship>>, StoreError> {
    let old: Posting = serde_json::from_str(posting_json)?;
    Ok(old
        .facts
        .sponsorship
        .filter(|f| f.source == FactSource::Feed))
}

/// Increments a posting's miss count, closing it at the threshold.
/// Returns whether it closed.
fn take_miss(conn: &Connection, id: i64, missed: i64, now: i64) -> Result<bool, StoreError> {
    let missed = missed + 1;
    if missed >= MISSES_TO_CLOSE {
        conn.execute(
            "UPDATE postings SET missed_polls = ?2, closed_at = ?3 WHERE id = ?1",
            params![id, missed, now],
        )?;
        log_event(conn, "posting", id, Some("open"), "closed", now)?;
        Ok(true)
    } else {
        conn.execute(
            "UPDATE postings SET missed_polls = ?2 WHERE id = ?1",
            params![id, missed],
        )?;
        Ok(false)
    }
}

/// Links a new posting to the earliest posting of the same company, title
/// and location whose description is near-identical (§4.2).
fn link_repost(
    conn: &Connection,
    id: i64,
    company_norm: &str,
    title_norm: &str,
    location_norm: &str,
    sim: i64,
) -> Result<Option<i64>, StoreError> {
    let candidates: Vec<(i64, i64, Option<i64>)> = {
        let mut stmt = conn.prepare(
            "SELECT id, simhash, duplicate_of FROM postings
             WHERE company_norm = ?1 AND title_norm = ?2 AND location_norm = ?3 AND id != ?4
             ORDER BY first_seen, id",
        )?;
        let rows = stmt.query_map(params![company_norm, title_norm, location_norm, id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let original = candidates
        .into_iter()
        .find(|(_, other, _)| hamming(*other as u64, sim as u64) <= REPOST_SIMHASH_BITS)
        .map(|(other_id, _, root)| root.unwrap_or(other_id));
    if let Some(original) = original {
        conn.execute(
            "UPDATE postings SET duplicate_of = ?2 WHERE id = ?1",
            params![id, original],
        )?;
    }
    Ok(original)
}

#[cfg(test)]
mod tests {
    use gantry_core::facts::PostingFacts;

    use super::*;

    fn posting(board: &str, job: &str, title: &str, description: &str) -> Posting {
        Posting {
            key: PostingKey {
                ats: Ats::Greenhouse,
                board_token: board.into(),
                job_id: job.into(),
            },
            company_name: "Northwind Labs".into(),
            title: title.into(),
            url: format!("https://job-boards.greenhouse.io/{board}/jobs/{job}"),
            locations_raw: vec!["Seattle, WA".into()],
            locations: vec![],
            work_mode: None,
            job_type: None,
            published_at: None,
            description: description.into(),
            facts: PostingFacts::default(),
        }
    }

    const DESC: &str = "Build and operate the billing service. You will design APIs, write \
                        Rust and SQL, review code, and join the on-call rotation with the team.";

    #[test]
    fn upsert_tracks_new_unchanged_updated() {
        let store = Store::open_in_memory().unwrap();
        let p = posting("nw", "1", "Backend Engineer", DESC);
        let a = store.upsert_posting(&p, None, "board", &p.url, 10).unwrap();
        assert_eq!(a.status, UpsertStatus::New);
        let b = store.upsert_posting(&p, None, "board", &p.url, 20).unwrap();
        assert_eq!((b.id, b.status), (a.id, UpsertStatus::Unchanged));
        let mut changed = p.clone();
        changed.description.push_str(" Hybrid three days a week.");
        let c = store
            .upsert_posting(&changed, None, "board", &p.url, 30)
            .unwrap();
        assert_eq!(c.status, UpsertStatus::Updated);
    }

    #[test]
    fn same_key_from_two_sources_is_one_posting() {
        let store = Store::open_in_memory().unwrap();
        let p = posting("nw", "1", "Backend Engineer", DESC);
        let a = store
            .upsert_posting(&p, None, "greenhouse", &p.url, 10)
            .unwrap();
        let id = store
            .add_source_if_exists(&p.key, "feed:simplify-new-grad", "https://x.test", 11)
            .unwrap();
        assert_eq!(id, Some(a.id));
        let rows = store.list_postings(&ListFilter::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sources, ["feed:simplify-new-grad", "greenhouse"]);
    }

    #[test]
    fn repost_links_to_the_original() {
        let store = Store::open_in_memory().unwrap();
        let original = posting("nw", "1", "Backend Engineer", DESC);
        let a = store
            .upsert_posting(&original, None, "board", "u", 10)
            .unwrap();
        let repost = posting("nw", "2", "Backend Engineer", &format!("{DESC} "));
        let b = store
            .upsert_posting(&repost, None, "board", "u", 20)
            .unwrap();
        assert_eq!(b.repost_of, Some(a.id));
        let third = posting("nw", "3", "Backend Engineer", DESC);
        let c = store
            .upsert_posting(&third, None, "board", "u", 30)
            .unwrap();
        assert_eq!(c.repost_of, Some(a.id), "chains resolve to the root");
        let different = posting(
            "nw",
            "4",
            "Backend Engineer",
            "Completely different role text here.",
        );
        let d = store
            .upsert_posting(&different, None, "board", "u", 40)
            .unwrap();
        assert_eq!(d.repost_of, None);
    }

    #[test]
    fn board_closes_posting_after_two_misses_and_reopens() {
        let store = Store::open_in_memory().unwrap();
        let p = posting("nw", "1", "Backend Engineer", DESC);
        store.upsert_posting(&p, None, "board", "u", 10).unwrap();
        let empty = HashSet::new();
        assert_eq!(
            store
                .reconcile_board(Ats::Greenhouse, "nw", Some(&empty), 20)
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .reconcile_board(Ats::Greenhouse, "nw", Some(&empty), 30)
                .unwrap(),
            1
        );
        assert!(
            store
                .list_postings(&ListFilter::default())
                .unwrap()
                .is_empty()
        );
        let back = store.upsert_posting(&p, None, "board", "u", 40).unwrap();
        assert_eq!(back.status, UpsertStatus::Reopened);
    }

    #[test]
    fn one_miss_then_seen_resets() {
        let store = Store::open_in_memory().unwrap();
        let p = posting("nw", "1", "Backend Engineer", DESC);
        store.upsert_posting(&p, None, "board", "u", 10).unwrap();
        store
            .reconcile_board(Ats::Greenhouse, "nw", Some(&HashSet::new()), 20)
            .unwrap();
        store.upsert_posting(&p, None, "board", "u", 30).unwrap();
        assert_eq!(
            store
                .reconcile_board(Ats::Greenhouse, "nw", Some(&HashSet::new()), 40)
                .unwrap(),
            0
        );
    }

    #[test]
    fn not_modified_after_a_miss_closes() {
        let store = Store::open_in_memory().unwrap();
        let gone = posting("nw", "1", "Backend Engineer", DESC);
        let kept = posting("nw", "2", "Data Engineer", "Other text entirely.");
        store.upsert_posting(&gone, None, "board", "u", 10).unwrap();
        store.upsert_posting(&kept, None, "board", "u", 10).unwrap();
        store.upsert_posting(&kept, None, "board", "u", 20).unwrap();
        let listed = HashSet::from(["2".to_owned()]);
        store
            .reconcile_board(Ats::Greenhouse, "nw", Some(&listed), 20)
            .unwrap();
        assert_eq!(
            store
                .reconcile_board(Ats::Greenhouse, "nw", None, 30)
                .unwrap(),
            1,
            "the unchanged body still lacks job 1"
        );
        let open = store.open_postings().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].posting.key.job_id, "2");
    }

    #[test]
    fn feed_reconciles_only_postings_it_alone_lists() {
        let store = Store::open_in_memory().unwrap();
        let stand_in = posting("nw", "1", "Backend Engineer", "");
        let shared = posting("nw", "2", "Data Engineer", "");
        store
            .upsert_posting(&stand_in, None, "feed:x", "u", 10)
            .unwrap();
        store
            .upsert_posting(&shared, None, "feed:x", "u", 10)
            .unwrap();
        store
            .upsert_posting(&shared, None, "greenhouse", "u", 10)
            .unwrap();
        let empty = HashSet::new();
        store.reconcile_feed("feed:x", Some(&empty), 20).unwrap();
        assert_eq!(store.reconcile_feed("feed:x", Some(&empty), 30).unwrap(), 1);
        let open = store.open_postings().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].posting.key.job_id, "2");
    }

    #[test]
    fn listing_again_resets_misses_for_postings_never_reupserted() {
        let store = Store::open_in_memory().unwrap();
        let stand_in = posting("nw", "1", "Backend Engineer", "");
        store
            .upsert_posting(&stand_in, None, "feed:x", "u", 10)
            .unwrap();
        let listed = HashSet::from([stand_in.key.clone()]);
        let empty = HashSet::new();
        store.reconcile_feed("feed:x", Some(&empty), 20).unwrap();
        store.reconcile_feed("feed:x", Some(&listed), 30).unwrap();
        assert_eq!(store.reconcile_feed("feed:x", None, 40).unwrap(), 0);
        assert_eq!(store.reconcile_feed("feed:x", Some(&empty), 50).unwrap(), 0);
        assert_eq!(store.open_postings().unwrap().len(), 1);
    }

    #[test]
    fn empty_descriptions_are_not_reposts() {
        let store = Store::open_in_memory().unwrap();
        let a = posting("nw", "1", "Backend Engineer", "");
        let b = posting("nw", "2", "Backend Engineer", "");
        store.upsert_posting(&a, None, "feed:x", "u", 10).unwrap();
        let out = store.upsert_posting(&b, None, "feed:x", "u", 20).unwrap();
        assert_eq!(out.repost_of, None);
    }

    #[test]
    fn feed_sponsorship_survives_board_refresh() {
        let store = Store::open_in_memory().unwrap();
        let p = posting("nw", "1", "Backend Engineer", DESC);
        let id = store
            .upsert_posting(&p, None, "greenhouse", "u", 10)
            .unwrap()
            .id;
        let fact = Fact {
            value: Sponsorship::NotOffered,
            source: FactSource::Feed,
            evidence: "Does Not Offer Sponsorship".into(),
        };
        store.set_feed_sponsorship(id, &fact).unwrap();
        store
            .upsert_posting(&p, None, "greenhouse", "u", 20)
            .unwrap();
        let open = store.open_postings().unwrap();
        assert_eq!(open[0].posting.facts.sponsorship, Some(fact));
    }
}
