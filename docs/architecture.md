# Architecture

The plan of record is `gantry_plan.md` (outside this repository for now).
This file describes what exists in the code.

## Crates

```text
gantry-cli ──► gantry-cmd ──► gantry-discovery ──► gantry-store ──► gantry-core
                    │                │                   │
                    └────────────────┴──► gantry-platform ◄┘
```

| Crate | Holds | Rule |
|---|---|---|
| `gantry-core` | Posting model, `search.toml` schema, bank items, location parsing, fact detectors, filters, SimHash | No I/O. No occupation-specific values. |
| `gantry-platform` | Paths, OS keyring, scheduler, browser lookup, atomic file replace, permissions | The only crate with `cfg(target_os)` / `cfg(unix)` / `cfg(windows)` (checked in CI). |
| `gantry-store` | TOML snapshot loading and atomic saves; SQLite (WAL) with numbered forward-only migrations | Migrations are append-only. |
| `gantry-discovery` | Board API adapters, list feeds, HN, slug probing, URL resolver, offline geocoder, polite fetcher, the discovery run | Every request goes through `http::Fetcher`. |
| `gantry-cmd` | One function per operation, typed outputs with JSON Schemas | The only API the CLI (and later the app) calls. |
| `gantry-cli` | Binary `gantry` | Argument parsing and output formatting only (checked in CI). |

## A discovery run

1. Register boards: seed list for the user's occupation packs
   (`data/companies/`), the user's `companies.toml`, URLs added with
   `gantry url add`.
2. Read enabled list feeds and the HN "Who is hiring?" thread, concurrently.
   Feed listings on a known ATS register that board; others become
   external postings. HN comments contribute company names and links only.
3. Probe board slugs for up to 200 queued company names (cached; misses are
   retried after 30 days). A Greenhouse hit must carry the same company name.
4. Poll every board, one task per API host. Conditional GET; a 304 marks
   every open posting on that board as seen.
5. Attach feed listings to the board postings they point at.
6. Evaluate every open posting against `search.toml` and the verified
   banks; store each check's verdict and reason.

## Politeness

One request per second per host (more if `Crawl-delay` says so),
`robots.txt` fetched and honored for every host, conditional GET with
stored `ETag`/`Last-Modified`, exponential backoff with `Retry-After` on
429 and 5xx, and a `User-Agent` naming Gantry. An unreachable or 5xx
`robots.txt` blocks the host for the run.

## Dedup and closing

- Primary key `(ats, board_token, ats_job_id)`; the same job from several
  sources is one row with several `posting_sources`.
- A new posting with the same company, title and location as an earlier
  one, and a description within 3 SimHash bits, gets `duplicate_of` set to
  the earliest such posting.
- A posting missing from two consecutive successful polls of its board
  (or feed, for external postings) is closed. It reopens if it reappears.

## Filters

Every section of `search.toml` defaults to "any". A posting is excluded
only by a failing check in a `hard` section (title exclude terms and the
company block list are always hard). Missing information yields `unknown`,
which never excludes. Bank answers count only when `verified = true`.
