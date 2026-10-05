# Gantry

Gantry is a free, open-source (MIT) desktop app for job seekers in any
field. You describe the work you want once. Every night Gantry finds
matching postings and stages a complete application for each; you review
them, and one click per application presses the employer's own Submit
button. Gantry never submits anything you have not clicked through.

## Status

Early development. Milestones 0 and 1 are in place:

- `gantry doctor` checks the machine;
- `gantry run --discover-only` finds postings on Greenhouse, Lever and
  Ashby boards, the SimplifyJobs early-career lists and the Hacker News
  "Who is hiring?" thread, deduplicates them, and filters them against
  your `search.toml`;
- `gantry postings list` shows what passed and why the rest did not.

Nothing is staged or submitted yet. There is no GUI yet; onboarding
(writing `search.toml` for you) arrives in M2.

Linux is supported. Windows and macOS build in CI and are ported later.

## Roadmap

Each milestone ends with something usable on its own.

| Milestone | Scope | Status |
|---|---|---|
| M0 | Workspace, platform layer, CI on Linux, Windows and macOS, TOML and SQLite storage, `gantry doctor` | Done |
| M1 | Discovery from board APIs, list feeds and HN; dedup, offline geocoding, filters from `search.toml` | Done |
| M2 | Question bank and technology pack, Tauri app shell, onboarding, resume import with per-item verification | Next |
| M3 | Model layer: local and API providers, schema-bound extraction, injection defenses, scoring, `gantry eval` | Planned |
| M4 | Resume rendering with Typst, one-page fitting, text read-back check | Planned |
| M5 | Staging: answers from the verified bank, cited drafts, review flags, nightly cap and per-company cooldown | Planned |
| M6 | Review queue and browser: fill on open, read-back diff, one click to submit; Greenhouse, then Lever, then Ashby | Planned |
| M7 | Scheduled nightly runs, desktop notification, saving review answers back to the bank | Planned |
| M7b | More sources: other ATS boards, remote-job APIs, USAJOBS, big-tech career sites | Planned |
| M8 | Linux packages (AppImage, deb, rpm, AUR), signed releases, 0.1.0 | Planned |
| M9 | Windows and macOS ports and installers | Planned |
| M10 | The next ATS by observed posting share, after a terms check | Planned |
| M11+ | Occupation packs: finance, healthcare, retail and hospitality, then trades, education, logistics, government | Planned |

Version 1.0 is M9 finished with the technology pack stable. Packs without
a reviewer from that field ship marked "unreviewed".

Some sites' terms prohibit automated access: Workday, Handshake, LinkedIn,
Indeed, Glassdoor, ZipRecruiter and Wellfound. Gantry never sends them a
request. Their postings get every answer prepared for you to paste in
yourself (manual paste mode, M6).

The full plan, with decisions and the reasons for them, is in
[`docs/plan.md`](docs/plan.md).

## Build

Requires Rust 1.95 or newer and a C compiler (for the bundled SQLite).

    cargo build --release
    ./target/release/gantry doctor

## Try it with a fictional profile

    gantry --config-dir fixtures/profiles/student --data-dir /tmp/gantry-demo \
        run --discover-only
    gantry --config-dir fixtures/profiles/student --data-dir /tmp/gantry-demo \
        postings list

This contacts the real sources listed below. The fixture tests
(`cargo test`) use saved responses and make no network requests.

## Privacy

Gantry has no telemetry. In M1 it contacts only:

- `boards-api.greenhouse.io`, `api.lever.co`, `api.eu.lever.co`,
  `api.ashbyhq.com` (public job board APIs);
- `raw.githubusercontent.com` (list feeds you enable in `settings.toml`);
- `hacker-news.firebaseio.com` (only if `hacker_news = true`).

Your home location is geocoded offline from bundled data; it never leaves
the machine. Files Gantry writes, including the database, are readable
only by your user; `gantry doctor` warns about profile files you created
that other users can read.

## Documentation

- [`docs/plan.md`](docs/plan.md): plan of record, milestones, decisions
- [`docs/cli.md`](docs/cli.md): commands, JSON output, exit codes
- [`docs/architecture.md`](docs/architecture.md): crates and the discovery run
- [`fixtures/profiles/README.md`](fixtures/profiles/README.md): fictional test profiles

## License

MIT; see [LICENSE](LICENSE). Place data in `data/geo/` is from GeoNames
under CC BY 4.0; see [`data/geo/README.md`](data/geo/README.md).
