//! Variant harvest driver (m2a question-bank design §8). Developer tool: it
//! samples the seed Greenhouse boards with `?questions=true` and writes
//! `fixtures/questions/harvest-<today>.json`. It never touches
//! `data/questions/`; a human copies reviewed variants in a PR.
//!
//! ```text
//! cargo run -p gantry-discovery --example harvest_questions -- \
//!     [--jobs-per-board N] [--out PATH]
//! ```
//!
//! Requests go through `Fetcher`: host allowlist, robots.txt, 1 req/s.

use std::path::PathBuf;
use std::process::ExitCode;

use gantry_core::Ats;
use gantry_discovery::bundled::seed_companies;
use gantry_discovery::harvest::{
    BoardFailure, HarvestReport, JobSample, parse_job_questions, pick_job_ids,
};
use gantry_discovery::http::{Fetched, Fetcher, Politeness, ReqwestTransport};
use gantry_store::Store;

const DEFAULT_JOBS_PER_BOARD: usize = 4;

struct Args {
    jobs_per_board: usize,
    out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        jobs_per_board: DEFAULT_JOBS_PER_BOARD,
        out: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--jobs-per-board" => {
                args.jobs_per_board = value("--jobs-per-board")?
                    .parse()
                    .map_err(|e| format!("--jobs-per-board: {e}"))?;
            }
            "--out" => args.out = Some(PathBuf::from(value("--out")?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

/// The body of a 200 response; every other outcome is an error message.
async fn get_body(fetcher: &Fetcher<'_>, url: &str) -> Result<Vec<u8>, String> {
    match fetcher.get(url, false).await {
        Ok(Fetched::Body { body, .. }) => Ok(body),
        Ok(Fetched::NotFound) => Err("HTTP 404".to_owned()),
        Ok(Fetched::NotModified) => Err("unexpected 304".to_owned()),
        Ok(Fetched::Disallowed) => Err("disallowed by robots.txt".to_owned()),
        Err(e) => Err(e.to_string()),
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let today = jiff::Zoned::now().date().to_string();
    let out = args.out.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/questions")
            .join(format!("harvest-{today}.json"))
    });

    let boards: Vec<String> = seed_companies(&["technology".to_owned()])
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|c| c.ats == Ats::Greenhouse)
        .map(|c| c.token)
        .collect();

    let transport = ReqwestTransport::new()?;
    let store = Store::open_in_memory().map_err(|e| e.to_string())?;
    let fetcher = Fetcher::new(
        &transport,
        &store,
        Politeness::default(),
        jiff::Timestamp::now().as_second(),
    );

    let mut samples = Vec::new();
    let mut failures = Vec::new();
    for token in &boards {
        let mut fail = |error: String| {
            eprintln!("{token}: {error}");
            failures.push(BoardFailure {
                token: token.clone(),
                error,
            });
        };
        let list_url = format!("https://boards-api.greenhouse.io/v1/boards/{token}/jobs");
        let ids = match get_body(&fetcher, &list_url)
            .await
            .and_then(|body| pick_job_ids(&body, args.jobs_per_board).map_err(|e| e.message))
        {
            Ok(ids) => ids,
            Err(error) => {
                fail(error);
                continue;
            }
        };
        for id in ids {
            let url = format!(
                "https://boards-api.greenhouse.io/v1/boards/{token}/jobs/{id}?questions=true"
            );
            match get_body(&fetcher, &url)
                .await
                .and_then(|body| parse_job_questions(&body).map_err(|e| e.message))
            {
                Ok(questions) => samples.push(JobSample {
                    token: token.clone(),
                    job_id: id.to_string(),
                    questions,
                }),
                Err(error) => fail(format!("job {id}: {error}")),
            }
        }
    }

    let report = HarvestReport::new(today, &samples, failures);
    let mut json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    json.push('\n');
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&out, json).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!(
        "harvested {} questions from {} jobs on {} of {} boards ({} failures) -> {}",
        report.questions.len(),
        report.jobs_sampled,
        report.boards_sampled,
        boards.len(),
        report.boards_failed.len(),
        out.display()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("harvest_questions: {e}");
            ExitCode::FAILURE
        }
    }
}
