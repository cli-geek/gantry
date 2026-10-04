use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use gantry_core::Ats;
use gantry_core::text::normalize;
use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};

use crate::{CompanyEntry, StoreError};

/// Numbered, forward-only migrations. Append; never edit a released one.
const MIGRATIONS: &[M<'static>] = &[M::up(include_str!("../migrations/0001_discovery.sql"))];

/// Probe misses are re-checked after this long; hits never expire.
const PROBE_MISS_TTL_SECS: i64 = 30 * 86_400;

/// The run-state database (§9.2). One connection behind a mutex: the
/// pipeline's writes are short, and WAL lets the app read concurrently
/// from its own connection.
#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

/// Conditional-GET validators last returned for a URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyRow {
    pub id: i64,
    pub name: String,
    pub ats: Ats,
    pub token: String,
    pub source: String,
}

impl Store {
    pub const LATEST_SCHEMA_VERSION: usize = MIGRATIONS.len();

    /// Opens (creating if needed) and migrates the database at `path`. The
    /// parent directory must exist.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        gantry_platform::restrict_file(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.busy_timeout(Duration::from_secs(10))?;
        // `journal_mode` returns the resulting mode as a row.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Migrations::from_slice(MIGRATIONS).to_latest(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }

    pub fn journal_mode(&self) -> Result<String, StoreError> {
        Ok(self
            .conn()?
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))?)
    }

    pub fn schema_version(&self) -> Result<usize, StoreError> {
        let v: i64 = self
            .conn()?
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        Ok(usize::try_from(v).unwrap_or(0))
    }

    /// `PRAGMA integrity_check`: `["ok"]` when the file is sound.
    pub fn integrity_check(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("PRAGMA integrity_check")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn begin_run(&self, kind: &str, now: i64) -> Result<i64, StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO runs (kind, started_at) VALUES (?1, ?2)",
            params![kind, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn finish_run(
        &self,
        run_id: i64,
        now: i64,
        discovered: i64,
        errors_json: &str,
        report_json: &str,
    ) -> Result<(), StoreError> {
        self.conn()?.execute(
            "UPDATE runs SET finished_at = ?2, discovered = ?3, errors_json = ?4, report_json = ?5
             WHERE id = ?1",
            params![run_id, now, discovered, errors_json, report_json],
        )?;
        Ok(())
    }

    /// Adds a company board if new. Returns its id and whether it was added.
    pub fn upsert_company(
        &self,
        entry: &CompanyEntry,
        source: &str,
        now: i64,
    ) -> Result<(i64, bool), StoreError> {
        let conn = self.conn()?;
        let existing: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, name FROM companies WHERE ats = ?1 AND board_token = ?2",
                params![entry.ats.as_str(), entry.token],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, name)) = existing {
            // A board added from a bare URL is named by its token until a
            // source that knows the company's name lists it.
            if name == entry.token && entry.name != entry.token {
                conn.execute(
                    "UPDATE companies SET name = ?2 WHERE id = ?1",
                    params![id, entry.name],
                )?;
            }
            if let Some(staffing) = entry.staffing_agency {
                conn.execute(
                    "UPDATE companies SET staffing_agency = ?2 WHERE id = ?1",
                    params![id, staffing],
                )?;
            }
            return Ok((id, false));
        }
        conn.execute(
            "INSERT INTO companies (name, ats, board_token, source, staffing_agency, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entry.name,
                entry.ats.as_str(),
                entry.token,
                source,
                entry.staffing_agency,
                now
            ],
        )?;
        Ok((conn.last_insert_rowid(), true))
    }

    pub fn companies(&self) -> Result<Vec<CompanyRow>, StoreError> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id, name, ats, board_token, source FROM companies ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(CompanyRow {
                id: r.get(0)?,
                name: r.get(1)?,
                ats: ats_at(r, 2)?,
                token: r.get(3)?,
                source: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn record_poll(
        &self,
        company_id: i64,
        now: i64,
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        self.conn()?.execute(
            "UPDATE companies SET last_polled_at = ?2, last_poll_error = ?3 WHERE id = ?1",
            params![company_id, now, error],
        )?;
        Ok(())
    }

    pub fn http_validators(&self, url: &str) -> Result<Option<HttpValidators>, StoreError> {
        Ok(self
            .conn()?
            .query_row(
                "SELECT etag, last_modified FROM http_validators WHERE url = ?1",
                [url],
                |r| {
                    Ok(HttpValidators {
                        etag: r.get(0)?,
                        last_modified: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn save_http_validators(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        now: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        if etag.is_none() && last_modified.is_none() {
            conn.execute("DELETE FROM http_validators WHERE url = ?1", [url])?;
        } else {
            conn.execute(
                "INSERT INTO http_validators (url, etag, last_modified, checked_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (url) DO UPDATE SET etag = ?2, last_modified = ?3, checked_at = ?4",
                params![url, etag, last_modified, now],
            )?;
        }
        Ok(())
    }

    /// Records a company name seen in some source for later board probing.
    pub fn queue_probe_name(&self, name: &str, source: &str, now: i64) -> Result<(), StoreError> {
        let norm = normalize(name);
        if norm.is_empty() {
            return Ok(());
        }
        self.conn()?.execute(
            "INSERT OR IGNORE INTO probe_names (name_norm, name, source, added_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![norm, name.trim(), source, now],
        )?;
        Ok(())
    }

    /// Names never probed, then misses older than the retry interval,
    /// oldest first.
    pub fn probe_batch(&self, now: i64, limit: usize) -> Result<Vec<String>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT name FROM probe_names
             WHERE probed_at IS NULL OR (found = 0 AND probed_at < ?1)
             ORDER BY probed_at IS NOT NULL, added_at, name_norm
             LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![now - PROBE_MISS_TTL_SECS, limit], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn finish_probe(&self, name: &str, found: bool, now: i64) -> Result<(), StoreError> {
        self.conn()?.execute(
            "UPDATE probe_names SET probed_at = ?2, found = ?3 WHERE name_norm = ?1",
            params![normalize(name), now, found],
        )?;
        Ok(())
    }

    /// A cached probe result still in force.
    pub fn cached_probe(&self, slug: &str, ats: Ats, now: i64) -> Result<Option<bool>, StoreError> {
        Ok(self
            .conn()?
            .query_row(
                "SELECT hit FROM board_probes
                 WHERE slug = ?1 AND ats = ?2 AND (hit = 1 OR checked_at >= ?3)",
                params![slug, ats.as_str(), now - PROBE_MISS_TTL_SECS],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn record_probe(
        &self,
        slug: &str,
        ats: Ats,
        hit: bool,
        now: i64,
    ) -> Result<(), StoreError> {
        self.conn()?.execute(
            "INSERT INTO board_probes (slug, ats, hit, checked_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (slug, ats) DO UPDATE SET hit = ?3, checked_at = ?4",
            params![slug, ats.as_str(), hit, now],
        )?;
        Ok(())
    }

    pub fn is_processed(&self, source: &str, item_id: &str) -> Result<bool, StoreError> {
        Ok(self
            .conn()?
            .query_row(
                "SELECT 1 FROM source_items WHERE source = ?1 AND item_id = ?2",
                params![source, item_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn mark_processed(&self, source: &str, item_id: &str, now: i64) -> Result<(), StoreError> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO source_items (source, item_id, processed_at) VALUES (?1, ?2, ?3)",
            params![source, item_id, now],
        )?;
        Ok(())
    }

    /// Returns `false` if the URL was already added.
    pub fn add_manual_url(
        &self,
        url: &str,
        resolved_json: &str,
        now: i64,
    ) -> Result<bool, StoreError> {
        Ok(self.conn()?.execute(
            "INSERT OR IGNORE INTO manual_urls (url, resolved_json, added_at) VALUES (?1, ?2, ?3)",
            params![url, resolved_json, now],
        )? == 1)
    }

    /// `(url, resolved_json)` pairs in the order added.
    pub fn manual_urls(&self) -> Result<Vec<(String, String)>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT url, resolved_json FROM manual_urls ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// Reads an `ats` column; rows are written only from `Ats::as_str`.
pub(crate) fn ats_at(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Ats> {
    let text: String = r.get(i)?;
    Ats::parse(&text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            i,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(format!("unknown ats \"{text}\""))),
        )
    })
}

pub(crate) fn log_event(
    conn: &Connection,
    entity: &str,
    entity_id: i64,
    from_state: Option<&str>,
    to_state: &str,
    now: i64,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO events (entity, entity_id, from_state, to_state, at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![entity, entity_id, from_state, to_state, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(token: &str) -> CompanyEntry {
        CompanyEntry {
            name: "Northwind Labs".into(),
            ats: Ats::Greenhouse,
            token: token.into(),
            staffing_agency: None,
        }
    }

    #[test]
    fn file_database_uses_wal_and_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gantry.db");
        let store = Store::open(&path).unwrap();
        assert_eq!(store.journal_mode().unwrap(), "wal");
        assert_eq!(
            store.schema_version().unwrap(),
            Store::LATEST_SCHEMA_VERSION
        );
        assert_eq!(store.integrity_check().unwrap(), ["ok"]);
        drop(store);
        // Reopening an up-to-date database is a no-op.
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            Store::LATEST_SCHEMA_VERSION
        );
    }

    #[test]
    fn migrations_are_valid() {
        Migrations::from_slice(MIGRATIONS).validate().unwrap();
    }

    #[test]
    fn company_upsert_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let (id, added) = store
            .upsert_company(&entry("northwind"), "seed", 100)
            .unwrap();
        assert!(added);
        let (id2, added2) = store
            .upsert_company(&entry("northwind"), "probe", 200)
            .unwrap();
        assert_eq!(id, id2);
        assert!(!added2);
        let rows = store.companies().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "seed");
    }

    #[test]
    fn token_placeholder_name_is_replaced_by_a_real_one() {
        let store = Store::open_in_memory().unwrap();
        let mut bare = entry("northwind");
        bare.name = "northwind".into();
        store.upsert_company(&bare, "manual", 1).unwrap();
        store
            .upsert_company(&entry("northwind"), "feed:x", 2)
            .unwrap();
        assert_eq!(store.companies().unwrap()[0].name, "Northwind Labs");
        let mut other = entry("northwind");
        other.name = "Something Else".into();
        store.upsert_company(&other, "hn", 3).unwrap();
        assert_eq!(store.companies().unwrap()[0].name, "Northwind Labs");
    }

    #[test]
    fn probe_queue_and_cache() {
        let store = Store::open_in_memory().unwrap();
        store
            .queue_probe_name("Pinecrest Robotics, Inc.", "hn", 10)
            .unwrap();
        store
            .queue_probe_name("pinecrest robotics inc", "feed", 11)
            .unwrap();
        assert_eq!(
            store.probe_batch(20, 10).unwrap(),
            ["Pinecrest Robotics, Inc."]
        );
        store
            .finish_probe("Pinecrest Robotics, Inc.", false, 20)
            .unwrap();
        assert!(store.probe_batch(21, 10).unwrap().is_empty());
        let month_later = 20 + PROBE_MISS_TTL_SECS + 1;
        assert_eq!(store.probe_batch(month_later, 10).unwrap().len(), 1);

        store
            .record_probe("pinecrest", Ats::Lever, false, 20)
            .unwrap();
        assert_eq!(
            store.cached_probe("pinecrest", Ats::Lever, 21).unwrap(),
            Some(false)
        );
        assert_eq!(
            store
                .cached_probe("pinecrest", Ats::Lever, month_later)
                .unwrap(),
            None
        );
        store
            .record_probe("pinecrest", Ats::Ashby, true, 20)
            .unwrap();
        assert_eq!(
            store
                .cached_probe("pinecrest", Ats::Ashby, month_later)
                .unwrap(),
            Some(true)
        );
    }

    #[test]
    fn validators_round_trip_and_clear() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_http_validators("https://x.test/a", Some("W/\"1\""), None, 5)
            .unwrap();
        assert_eq!(
            store.http_validators("https://x.test/a").unwrap(),
            Some(HttpValidators {
                etag: Some("W/\"1\"".into()),
                last_modified: None
            })
        );
        store
            .save_http_validators("https://x.test/a", None, None, 6)
            .unwrap();
        assert_eq!(store.http_validators("https://x.test/a").unwrap(), None);
    }
}
