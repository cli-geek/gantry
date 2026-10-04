//! `gantry`: argv and JSON front end over `gantry-cmd` (§3.4). No logic of
//! its own beyond parsing arguments and formatting output.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use gantry_cmd::{
    CheckStatus, CmdError, Context, DiscoverReport, DoctorReport, ErrorBody, ErrorOutput,
    ExitCode as Code, Paths, PostingStatus, PostingSummary, PostingsQuery, Services, StatusFilter,
    UrlAdded, Verdict,
};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(
    name = "gantry",
    version,
    about = "Find job postings that match your search; stage applications for you to review.",
    after_help = "Exit codes: 0 ok, 1 error, 2 usage, 3 doctor checks failed, \
                  4 not available yet, 5 config file problem. See docs/cli.md."
)]
struct Cli {
    /// Print JSON matching the schema from `gantry schema <command>`.
    #[arg(long, global = true)]
    json: bool,
    /// Directory with search.toml and the other profile files.
    #[arg(long, global = true, env = "GANTRY_CONFIG_DIR", value_name = "DIR")]
    config_dir: Option<PathBuf>,
    /// Directory for the database.
    #[arg(long, global = true, env = "GANTRY_DATA_DIR", value_name = "DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check that Gantry can run on this machine.
    Doctor,
    /// Run the nightly pipeline. Only discovery exists so far.
    Run {
        /// Find, deduplicate and filter postings; stage nothing.
        #[arg(long)]
        discover_only: bool,
    },
    /// Discovered postings.
    #[command(subcommand)]
    Postings(PostingsCommand),
    /// Posting and board URLs to watch.
    #[command(subcommand)]
    Url(UrlCommand),
    /// Print the JSON Schema of a command's --json output.
    Schema {
        /// One of: doctor, run, postings-list, url-add, error. Omit to list.
        name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum PostingsCommand {
    /// Open postings that pass your filters, newest first.
    List {
        /// Show postings your filters excluded, with the reasons.
        #[arg(long, conflicts_with = "all")]
        filtered_out: bool,
        /// Show every posting.
        #[arg(long)]
        all: bool,
        /// Include postings no longer listed by their source.
        #[arg(long)]
        include_closed: bool,
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
}

#[derive(Debug, Subcommand)]
enum UrlCommand {
    /// Add a posting or board URL; the next run polls its board.
    Add { url: String },
}

fn main() -> ExitCode {
    let json_requested = std::env::args().any(|a| a == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if !e.use_stderr() => e.exit(),
        Err(e) => {
            if json_requested {
                print_json(&ErrorOutput {
                    error: ErrorBody {
                        kind: "usage".into(),
                        message: e.to_string(),
                    },
                });
            } else {
                let _ = e.print();
            }
            return ExitCode::from(Code::Usage as u8);
        }
    };
    init_logging();
    match run(&cli) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            if cli.json {
                print_json(&e.to_output());
            } else {
                eprintln!("gantry: {e}");
            }
            ExitCode::from(e.exit_code() as u8)
        }
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("GANTRY_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn context(cli: &Cli) -> Result<Context, CmdError> {
    let paths = match (&cli.config_dir, &cli.data_dir) {
        (Some(c), Some(d)) => Paths::new(c, d),
        (config, data) => {
            let os = Paths::from_os()?;
            Paths::new(
                config
                    .clone()
                    .unwrap_or_else(|| os.config_dir().to_path_buf()),
                data.clone().unwrap_or_else(|| os.data_dir().to_path_buf()),
            )
        }
    };
    Ok(Context::new(paths))
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("gantry: could not serialize output: {e}"),
    }
}

fn emit<T: Serialize>(cli: &Cli, value: &T, human: impl FnOnce(&T) -> String) {
    if cli.json {
        print_json(value);
    } else {
        print!("{}", human(value));
    }
}

fn run(cli: &Cli) -> Result<Code, CmdError> {
    match &cli.command {
        Command::Schema { name } => {
            match name {
                None => println!("{}", gantry_cmd::schema::names().join("\n")),
                Some(n) => {
                    let schema = gantry_cmd::schema::get(n).ok_or_else(|| {
                        CmdError::InvalidInput(format!(
                            "no schema named \"{n}\"; choose one of: {}",
                            gantry_cmd::schema::names().join(", ")
                        ))
                    })?;
                    print_json(&schema);
                }
            }
            Ok(Code::Ok)
        }
        Command::Doctor => {
            let ctx = context(cli)?;
            let report = gantry_cmd::doctor(&ctx, &Services::os());
            emit(cli, &report, format_doctor);
            Ok(if report.ok {
                Code::Ok
            } else {
                Code::ChecksFailed
            })
        }
        Command::Run { discover_only } => {
            if !discover_only {
                return Err(CmdError::NotAvailable {
                    what: "staging (`gantry run` without --discover-only)",
                    milestone: "M5",
                });
            }
            let ctx = context(cli)?;
            let runtime = tokio::runtime::Runtime::new().map_err(|e| {
                CmdError::Network(format!("could not start the async runtime: {e}"))
            })?;
            let started = std::time::Instant::now();
            let report = runtime.block_on(gantry_cmd::run_discover(&ctx, gantry_cmd::now()))?;
            emit(cli, &report, |r| format_run(r, started.elapsed().as_secs()));
            Ok(Code::Ok)
        }
        Command::Postings(PostingsCommand::List {
            filtered_out,
            all,
            include_closed,
            limit,
        }) => {
            let ctx = context(cli)?;
            let status = if *all {
                StatusFilter::All
            } else if *filtered_out {
                StatusFilter::FilteredOut
            } else {
                StatusFilter::Passed
            };
            let rows = gantry_cmd::list_postings(
                &ctx,
                PostingsQuery {
                    status,
                    include_closed: *include_closed,
                    limit: *limit,
                },
            )?;
            emit(cli, &rows, |r| format_postings(r));
            Ok(Code::Ok)
        }
        Command::Url(UrlCommand::Add { url }) => {
            let ctx = context(cli)?;
            let added = gantry_cmd::add_url(&ctx, url, gantry_cmd::now())?;
            emit(cli, &added, format_url_added);
            Ok(Code::Ok)
        }
    }
}

fn indent(text: &str, by: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                l.to_owned()
            } else {
                format!("{by}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_doctor(r: &DoctorReport) -> String {
    let mut out = format!(
        "gantry {} on {}\nconfig  {}\ndata    {}\n\n",
        r.version, r.os, r.config_dir, r.data_dir
    );
    for c in &r.checks {
        let status = match c.status {
            CheckStatus::Ok => "ok",
            CheckStatus::Warn => "warn",
            CheckStatus::Fail => "FAIL",
            CheckStatus::Skip => "skip",
        };
        out.push_str(&format!(
            "  {status:<5} {:<15} {}\n",
            c.name,
            indent(&c.detail, &" ".repeat(24))
        ));
    }
    if !r.ok {
        out.push_str("\nAt least one check failed; fix it before running Gantry.\n");
    }
    out
}

fn format_run(r: &DiscoverReport, seconds: u64) -> String {
    let p = &r.postings;
    let b = &r.boards;
    let mut out = format!("Discovery run {} finished in {seconds} s.\n", r.run_id);
    out.push_str(&format!(
        "  boards    {} polled, {} unchanged, {} not found, {} failed; {} new boards\n",
        b.polled, b.unchanged, b.not_found, b.failed, r.boards_added
    ));
    for f in &r.feeds {
        out.push_str(
            &format!(
                "  feed      {} {:?} ({} listings)\n",
                f.source, f.outcome, f.items
            )
            .to_lowercase(),
        );
    }
    if let Some(h) = &r.hacker_news {
        out.push_str(
            &format!("  hn        {:?} ({} new comments)\n", h.outcome, h.items).to_lowercase(),
        );
    }
    out.push_str(&format!(
        "  probing   {} names, {} boards found\n",
        r.probes.names, r.probes.boards_found
    ));
    out.push_str(&format!(
        "  postings  {} seen: {} new, {} updated, {} reopened, {} closed, {} reposts\n",
        p.seen, p.new, p.updated, p.reopened, p.closed, p.reposts
    ));
    out.push_str(&format!(
        "  filters   {} open: {} pass, {} filtered out; {} with a location that could not be placed\n",
        p.open, p.passed, p.filtered_out, p.unresolved_locations
    ));
    if let Some(home) = &r.home {
        out.push_str(&format!("  home      {home}\n"));
    }
    for w in &r.warnings {
        out.push_str(&format!("warning: {w}\n"));
    }
    if r.error_count > 0 {
        out.push_str(&format!("{} source errors:\n", r.error_count));
        for e in r.errors.iter().take(10) {
            out.push_str(&format!("  {}: {}\n", e.source, e.message));
        }
        if r.error_count > 10 {
            out.push_str("  (all errors: --json)\n");
        }
    }
    out.push_str("Next: gantry postings list\n");
    out
}

fn date(ts: i64) -> String {
    jiff::Timestamp::from_second(ts)
        .map(|t| t.strftime("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn format_postings(rows: &[PostingSummary]) -> String {
    if rows.is_empty() {
        return "No postings. Run `gantry run --discover-only` first, or try --all.\n".into();
    }
    let mut out = String::new();
    for r in rows {
        let status = match r.status {
            PostingStatus::Passed => "pass",
            PostingStatus::FilteredOut => "out",
            PostingStatus::NotEvaluated => "new",
        };
        let posted = date(r.published_at.unwrap_or(r.first_seen));
        out.push_str(&format!(
            "#{:<6} {status:<4} {} — {}\n",
            r.id, r.company, r.title
        ));
        out.push_str(&format!(
            "        {} · {} · posted {posted}{}\n        {}\n",
            if r.location.is_empty() {
                "no location"
            } else {
                &r.location
            },
            r.sources.join(", "),
            r.duplicate_of
                .map(|d| format!(" · repost of #{d}"))
                .unwrap_or_default(),
            r.url
        ));
        for c in &r.checks {
            let mark = match c.verdict {
                Verdict::Fail if c.hard => "x",
                Verdict::Fail => "-",
                Verdict::Unknown => "?",
                Verdict::Pass => continue,
            };
            out.push_str(&format!(
                "        {mark} {}: {}\n",
                format!("{:?}", c.filter).to_lowercase(),
                c.detail
            ));
        }
    }
    out
}

fn format_url_added(a: &UrlAdded) -> String {
    let head = if a.added { "Added" } else { "Already added" };
    format!("{head}: {}\n{}\n", a.url, a.next)
}
