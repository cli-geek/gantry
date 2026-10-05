//! M1 exit criterion: discovery over saved responses produces a deduped,
//! filtered list for each fictional tech profile, and the non-tech guard
//! profile runs through the same code with no technology defaults.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gantry_cmd::{
    Context, DiscoverReport, Paths, PostingStatus, PostingSummary, PostingsQuery, StatusFilter,
    Verdict, add_url, list_postings, run_discover_with,
};
use gantry_discovery::fixture::FixtureTransport;
use gantry_discovery::http::Politeness;
use gantry_discovery::pipeline::SourceOutcome;

/// 2026-10-03T12:00:00Z
const NOW: i64 = 1_791_028_800;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fast() -> Politeness {
    Politeness {
        min_interval: Duration::ZERO,
        retry_base: Duration::ZERO,
        max_attempts: 2,
    }
}

struct Fixture {
    ctx: Context,
    transport: FixtureTransport,
    _data: tempfile::TempDir,
}

impl Fixture {
    fn new(profile: &str) -> Self {
        let data = tempfile::tempdir().unwrap();
        let config = repo().join("fixtures/profiles").join(profile);
        assert!(config.is_dir(), "{}", config.display());
        Self {
            ctx: Context::new(Paths::new(config, data.path())),
            transport: FixtureTransport::load(&repo().join("fixtures/http/manifest.toml")).unwrap(),
            _data: data,
        }
    }

    async fn run(&self) -> DiscoverReport {
        run_discover_with(&self.ctx, &self.transport, fast(), NOW)
            .await
            .unwrap()
    }

    fn list(&self, status: StatusFilter) -> Vec<PostingSummary> {
        list_postings(
            &self.ctx,
            PostingsQuery {
                status,
                ..PostingsQuery::default()
            },
        )
        .unwrap()
    }

    fn passed(&self) -> BTreeSet<(String, String)> {
        self.list(StatusFilter::Passed)
            .into_iter()
            .map(|p| (p.company, p.title))
            .collect()
    }

    fn find(&self, company: &str, title: &str) -> Vec<PostingSummary> {
        self.list(StatusFilter::All)
            .into_iter()
            .filter(|p| p.company == company && p.title == title)
            .collect()
    }
}

fn set(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    items
        .iter()
        .map(|(c, t)| ((*c).to_owned(), (*t).to_owned()))
        .collect()
}

fn failed_filters(p: &PostingSummary) -> Vec<String> {
    p.checks
        .iter()
        .filter(|c| c.hard && c.verdict == Verdict::Fail)
        .map(|c| format!("{:?}", c.filter))
        .collect()
}

#[tokio::test]
async fn student_profile() {
    let f = Fixture::new("student");
    let report = f.run().await;
    assert_eq!(report.error_count, 0, "{:#?}", report.errors);
    assert_eq!(report.feeds.len(), 1);
    assert_eq!(report.feeds[0].outcome, SourceOutcome::Ok);
    assert!(report.hacker_news.is_none());
    assert_eq!(report.home.as_deref(), Some("Seattle, WA, US"));

    assert_eq!(
        f.passed(),
        set(&[
            ("Northwind Labs", "Software Engineer, New Grad"),
            ("Pinecrest Robotics", "Software Engineer, New Grad"),
            ("Quillstack", "Software Engineering Intern, Summer 2027"),
            ("Tidewater Analytics", "Junior Data Analyst"),
        ])
    );

    // The feed listing and the board posting are one posting.
    let intern = f.find("Quillstack", "Software Engineering Intern, Summer 2027");
    assert_eq!(intern.len(), 1);
    assert_eq!(intern[0].sources, ["feed:simplify-new-grad", "lever"]);

    // The repost is linked to the original.
    let new_grad = f.find("Northwind Labs", "Software Engineer, New Grad");
    assert_eq!(new_grad.len(), 2);
    let original = new_grad.iter().find(|p| p.duplicate_of.is_none()).unwrap();
    let repost = new_grad.iter().find(|p| p.duplicate_of.is_some()).unwrap();
    assert_eq!(repost.duplicate_of, Some(original.id));
    assert_eq!(original.sources, ["feed:simplify-new-grad", "greenhouse"]);
    assert_eq!(report.postings.reposts, 1);

    // Excluded for the reasons a student on a visa would expect.
    let harborview = &f.find("Harborview Health", "Software Engineer I")[0];
    assert_eq!(harborview.status, PostingStatus::FilteredOut);
    assert!(failed_filters(harborview).contains(&"Sponsorship".to_owned()));
    let nyc = &f.find("Northwind Labs", "Data Analyst")[0];
    assert!(failed_filters(nyc).contains(&"Location".to_owned()));
    assert!(failed_filters(nyc).contains(&"Sponsorship".to_owned()));
    let senior = &f.find("Northwind Labs", "Senior Backend Engineer")[0];
    assert!(failed_filters(senior).contains(&"Title".to_owned()));

    // The inactive listing never became a posting; the unlisted Ashby job
    // was dropped.
    assert!(f.find("Northwind Labs", "Old Closed Role").is_empty());
    assert!(f.find("Lumen Forge", "Unlisted Role").is_empty());

    // Every board request was preceded by that host's robots.txt.
    let requests = f.transport.requests();
    let first_gh = requests
        .iter()
        .position(|u| u.starts_with("https://boards-api.greenhouse.io/v1/"))
        .unwrap();
    let gh_robots = requests
        .iter()
        .position(|u| u == "https://boards-api.greenhouse.io/robots.txt")
        .unwrap();
    assert!(gh_robots < first_gh);
}

#[tokio::test]
async fn second_run_uses_conditional_get_and_keeps_postings_open() {
    let f = Fixture::new("student");
    let first = f.run().await;
    let second = f.run().await;
    assert_eq!(second.feeds[0].outcome, SourceOutcome::Unchanged);
    assert!(second.boards.unchanged >= 2, "{:#?}", second.boards);
    assert_eq!(second.postings.new, 0);
    assert_eq!(second.postings.closed, 0);
    assert_eq!(second.postings.open, first.postings.open);
    assert_eq!(second.postings.passed, first.postings.passed);
}

#[tokio::test]
async fn early_career_profile() {
    let f = Fixture::new("early-career");
    let report = f.run().await;
    assert_eq!(report.error_count, 0, "{:#?}", report.errors);

    assert_eq!(
        f.passed(),
        set(&[
            ("Cobalt Harbor Software", "Software Engineer, Backend"),
            ("Quillstack", "Backend Engineer"),
            // A feed listing in Seattle with no work mode: it may be remote,
            // so the location check is unknown and does not exclude it.
            ("Harborview Health", "Software Engineer I"),
        ])
    );

    // HN comment → company name → slug probe → Lever board → posting.
    let hn = report.hacker_news.unwrap();
    assert_eq!((hn.outcome, hn.items), (SourceOutcome::Ok, 3));
    assert!(report.probes.boards_found >= 1);
    let cobalt = &f.find("Cobalt Harbor Software", "Software Engineer, Backend")[0];
    assert_eq!(cobalt.sources, ["lever"]);

    // A Greenhouse board at Tidewater Analytics' slug belongs to another
    // company and was not adopted.
    assert!(
        !f.transport
            .requests()
            .iter()
            .any(|u| u.contains("/boards/tidewateranalytics/jobs"))
    );

    let stale = &f.find("Northwind Labs", "Backend Engineer")[0];
    assert!(failed_filters(stale).contains(&"Freshness".to_owned()));
    // "Platform Engineer" is only an unverified synonym.
    let platform = &f.find("Quillstack", "Platform Engineer")[0];
    assert!(failed_filters(platform).contains(&"Title".to_owned()));
}

#[tokio::test]
async fn career_changer_profile_with_manual_url() {
    let f = Fixture::new("career-changer");
    let added = add_url(
        &f.ctx,
        "https://jobs.ashbyhq.com/lumenforge/2c9b3d10-0000-4000-8000-000000000302/application",
        NOW,
    )
    .unwrap();
    assert!(added.added);
    let report = f.run().await;
    assert_eq!(report.error_count, 0, "{:#?}", report.errors);

    assert_eq!(
        f.passed(),
        set(&[
            // Added by URL only: the Ashby API gives no company name, so the
            // board token stands in until another source names it.
            ("lumenforge", "Data Scientist"),
            ("Tidewater Analytics", "Junior Data Analyst"),
        ])
    );
    let nyc = &f.find("Northwind Labs", "Data Analyst")[0];
    assert_eq!(failed_filters(nyc), ["Location"]);
}

#[tokio::test]
async fn nontech_guard_runs_with_no_technology_defaults() {
    let f = Fixture::new("nontech-guard");
    let report = f.run().await;
    assert_eq!(report.error_count, 0, "{:#?}", report.errors);
    assert!(report.feeds.is_empty());
    assert!(report.hacker_news.is_none());

    assert_eq!(
        f.passed(),
        set(&[
            ("BrightSmile Dental Group", "Dental Hygienist (Part-Time)"),
            ("BrightSmile Dental Group", "Registered Dental Hygienist"),
        ])
    );
    let hygienists = f.find("BrightSmile Dental Group", "Registered Dental Hygienist");
    assert_eq!(hygienists.len(), 2);
    let houston = hygienists
        .iter()
        .find(|p| p.location == "Houston, TX")
        .unwrap();
    assert_eq!(failed_filters(houston), ["Location"]);
    let engineer = &f.find(
        "BrightSmile Dental Group",
        "Software Engineer, Patient Scheduling",
    )[0];
    assert!(failed_filters(engineer).contains(&"Title".to_owned()));

    // License requirements are reported as not yet checked, never guessed.
    let rdh = hygienists
        .iter()
        .find(|p| p.location == "Austin, TX")
        .unwrap();
    assert!(
        rdh.checks
            .iter()
            .any(|c| format!("{:?}", c.filter) == "Licenses" && c.verdict == Verdict::Unknown)
    );

    // Only the user's own board was contacted: no seed boards, feeds,
    // Hacker News or probes.
    let hosts: BTreeSet<String> = f
        .transport
        .requests()
        .iter()
        .map(|u| u.split('/').nth(2).unwrap().to_owned())
        .collect();
    assert_eq!(
        hosts,
        BTreeSet::from(["boards-api.greenhouse.io".to_owned()])
    );
    assert_eq!(report.boards.polled, 1);
}
