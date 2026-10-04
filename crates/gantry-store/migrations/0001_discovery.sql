-- M1: discovery state (§9.2). Later milestones add their tables in new
-- numbered migrations; this file is never edited after release.

CREATE TABLE companies (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    ats             TEXT NOT NULL,
    board_token     TEXT NOT NULL,
    -- seed | user | probe | manual | feed:<id> | hn
    source          TEXT NOT NULL,
    staffing_agency INTEGER,
    added_at        INTEGER NOT NULL,
    last_polled_at  INTEGER,
    last_poll_error TEXT,
    UNIQUE (ats, board_token)
);

CREATE TABLE postings (
    id            INTEGER PRIMARY KEY,
    company_id    INTEGER REFERENCES companies (id),
    ats           TEXT NOT NULL,
    board_token   TEXT NOT NULL,
    ats_job_id    TEXT NOT NULL,
    company_name  TEXT NOT NULL,
    company_norm  TEXT NOT NULL,
    url           TEXT NOT NULL,
    title         TEXT NOT NULL,
    title_norm    TEXT NOT NULL,
    location_raw  TEXT NOT NULL,
    location_norm TEXT NOT NULL,
    -- First placed location, for map/sort; all locations are in posting_json.
    lat           REAL,
    lon           REAL,
    remote_mode   TEXT,
    -- The normalized posting (gantry_core::Posting) as JSON.
    posting_json  TEXT NOT NULL,
    content_hash  TEXT NOT NULL,
    -- u64 SimHash stored as its i64 bit pattern.
    simhash       INTEGER NOT NULL,
    published_at  INTEGER,
    first_seen    INTEGER NOT NULL,
    last_seen     INTEGER NOT NULL,
    -- Consecutive successful polls of its board that did not list it.
    missed_polls  INTEGER NOT NULL DEFAULT 0,
    closed_at     INTEGER,
    -- Earlier posting this one reposts (§4.2).
    duplicate_of  INTEGER REFERENCES postings (id),
    UNIQUE (ats, board_token, ats_job_id)
);

CREATE INDEX postings_repost ON postings (company_norm, title_norm);
CREATE INDEX postings_board ON postings (ats, board_token) WHERE closed_at IS NULL;

CREATE TABLE posting_sources (
    posting_id INTEGER NOT NULL REFERENCES postings (id) ON DELETE CASCADE,
    source     TEXT NOT NULL,
    source_url TEXT NOT NULL,
    first_seen INTEGER NOT NULL,
    seen_at    INTEGER NOT NULL,
    PRIMARY KEY (posting_id, source)
);

-- Latest deterministic evaluation per posting. Model-derived columns
-- (scores, injection signals) arrive with M3.
CREATE TABLE evaluations (
    posting_id          INTEGER PRIMARY KEY REFERENCES postings (id) ON DELETE CASCADE,
    filter_result       TEXT NOT NULL CHECK (filter_result IN ('pass', 'fail')),
    filter_reasons_json TEXT NOT NULL,
    facts_json          TEXT NOT NULL,
    created_at          INTEGER NOT NULL
);

CREATE TABLE events (
    id          INTEGER PRIMARY KEY,
    entity      TEXT NOT NULL,
    entity_id   INTEGER NOT NULL,
    from_state  TEXT,
    to_state    TEXT NOT NULL,
    detail_json TEXT,
    at          INTEGER NOT NULL
);

CREATE TABLE runs (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    discovered  INTEGER NOT NULL DEFAULT 0,
    staged      INTEGER NOT NULL DEFAULT 0,
    errors_json TEXT NOT NULL DEFAULT '[]',
    report_json TEXT
);

-- Conditional-GET validators per URL (§4.1 politeness). Bodies are not
-- cached: a 304 means the board is unchanged.
CREATE TABLE http_validators (
    url           TEXT PRIMARY KEY,
    etag          TEXT,
    last_modified TEXT,
    checked_at    INTEGER NOT NULL
);

-- Company names seen in any source, waiting for board-slug probing.
CREATE TABLE probe_names (
    name_norm TEXT PRIMARY KEY,
    name      TEXT NOT NULL,
    source    TEXT NOT NULL,
    added_at  INTEGER NOT NULL,
    probed_at INTEGER,
    found     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE board_probes (
    slug       TEXT NOT NULL,
    ats        TEXT NOT NULL,
    hit        INTEGER NOT NULL,
    checked_at INTEGER NOT NULL,
    PRIMARY KEY (slug, ats)
);

-- Items from append-only sources (HN comments) already processed.
CREATE TABLE source_items (
    source       TEXT NOT NULL,
    item_id      TEXT NOT NULL,
    processed_at INTEGER NOT NULL,
    PRIMARY KEY (source, item_id)
);

CREATE TABLE manual_urls (
    id            INTEGER PRIMARY KEY,
    url           TEXT NOT NULL UNIQUE,
    resolved_json TEXT NOT NULL,
    added_at      INTEGER NOT NULL
);
