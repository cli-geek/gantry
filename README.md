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
the machine. Profile files and the database are readable only by your user.

## Documentation

- [`docs/cli.md`](docs/cli.md): commands, JSON output, exit codes
- [`docs/architecture.md`](docs/architecture.md): crates and the discovery run
- [`fixtures/profiles/README.md`](fixtures/profiles/README.md): fictional test profiles

## License

MIT; see [LICENSE](LICENSE). Place data in `data/geo/` is from GeoNames
under CC BY 4.0; see [`data/geo/README.md`](data/geo/README.md).
