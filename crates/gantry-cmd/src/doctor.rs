//! `gantry doctor` (§13): checks that Gantry can run on this machine and
//! says what to do about each problem.

use gantry_discovery::bundled;
use gantry_discovery::geocode::gazetteer;
use gantry_platform::{
    BrowserLocator, OsBrowserLocator, OsScheduler, OsSecretStore, PlatformError, ScheduleStatus,
    Scheduler, SecretStore,
};
use gantry_store::{FileState, Store, check_files, load_snapshot};
use schemars::JsonSchema;
use serde::Serialize;

use crate::Context;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    /// Works, but something needs attention or a later feature will fail.
    Warn,
    /// Gantry cannot work until this is fixed.
    Fail,
    /// Not applicable yet on this OS or at this milestone.
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DoctorCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DoctorReport {
    pub version: String,
    pub os: String,
    pub config_dir: String,
    pub data_dir: String,
    /// `false` if any check failed.
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

/// OS services doctor probes; tests substitute fakes.
pub struct Services {
    pub secrets: Box<dyn Fn() -> Result<Box<dyn SecretStore>, PlatformError>>,
    pub scheduler: Box<dyn Scheduler>,
    pub browser: Box<dyn BrowserLocator>,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("scheduler", &self.scheduler)
            .field("browser", &self.browser)
            .finish_non_exhaustive()
    }
}

impl Services {
    pub fn os() -> Self {
        Self {
            secrets: Box::new(|| {
                OsSecretStore::open().map(|s| Box::new(s) as Box<dyn SecretStore>)
            }),
            scheduler: Box::new(OsScheduler),
            browser: Box::new(OsBrowserLocator),
        }
    }
}

fn check(name: &str, status: CheckStatus, detail: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.to_owned(),
        status,
        detail: detail.into(),
    }
}

pub fn doctor(ctx: &Context, services: &Services) -> DoctorReport {
    let checks = vec![
        config_files(ctx),
        search_profile(ctx),
        data_dir(ctx),
        database(ctx),
        secret_store(services),
        browser(services),
        scheduler(services),
        check(
            "model backend",
            CheckStatus::Skip,
            "no model backend is used before milestone M3",
        ),
        check(
            "resume fonts",
            CheckStatus::Skip,
            "resume rendering arrives in milestone M4",
        ),
    ];
    DoctorReport {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        os: gantry_platform::OS_NAME.to_owned(),
        config_dir: ctx.paths.config_dir().display().to_string(),
        data_dir: ctx.paths.data_dir().display().to_string(),
        ok: !checks.iter().any(|c| c.status == CheckStatus::Fail),
        checks,
    }
}

fn config_files(ctx: &Context) -> DoctorCheck {
    let dir = ctx.paths.config_dir();
    if !dir.exists() {
        return check(
            "config files",
            CheckStatus::Warn,
            format!(
                "{} does not exist; create search.toml there (onboarding arrives in M2)",
                dir.display()
            ),
        );
    }
    let states = check_files(dir);
    let invalid: Vec<String> = states
        .iter()
        .filter_map(|(path, s)| match s {
            FileState::Invalid(msg) => Some(format!("{}: {msg}", path.display())),
            _ => None,
        })
        .collect();
    if !invalid.is_empty() {
        return check("config files", CheckStatus::Fail, invalid.join("\n"));
    }
    let present: Vec<String> = states
        .iter()
        .filter(|(_, s)| *s == FileState::Valid)
        .filter_map(|(p, _)| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let mut warnings = Vec::new();
    if states
        .iter()
        .any(|(p, s)| *s == FileState::Missing && p.ends_with("search.toml"))
    {
        warnings.push("search.toml is missing, so every posting passes the filters".to_owned());
    }
    let exposed: Vec<String> = states
        .iter()
        .filter(|(p, s)| *s == FileState::Valid && gantry_platform::readable_by_others(p))
        .map(|(p, _)| p.display().to_string())
        .collect();
    if !exposed.is_empty() {
        warnings.push(format!(
            "other users can read: {}; run `chmod 600` on them",
            exposed.join(", ")
        ));
    }
    if warnings.is_empty() {
        check(
            "config files",
            CheckStatus::Ok,
            format!("valid: {}", present.join(", ")),
        )
    } else {
        check("config files", CheckStatus::Warn, warnings.join("\n"))
    }
}

fn search_profile(ctx: &Context) -> DoctorCheck {
    let Ok(snapshot) = load_snapshot(ctx.paths.config_dir()) else {
        return check(
            "search profile",
            CheckStatus::Skip,
            "config files do not parse",
        );
    };
    let mut problems = Vec::new();
    let loc = &snapshot.search.location;
    if let Some(home) = &loc.home {
        if gazetteer().locate(home).is_none() {
            problems.push(format!(
                "home \"{home}\" is not in the bundled place data; use a US ZIP code or \"City, Region\""
            ));
        }
    } else if loc.max_distance.is_some() {
        problems.push("max_distance is set but home is not".to_owned());
    }
    match bundled::feeds() {
        Ok(feeds) => {
            for id in &snapshot.settings.discovery.feeds {
                if !feeds.iter().any(|f| &f.id == id) {
                    let known: Vec<&str> = feeds.iter().map(|f| f.id.as_str()).collect();
                    problems.push(format!(
                        "settings.toml enables unknown feed \"{id}\"; known feeds: {}",
                        known.join(", ")
                    ));
                }
            }
        }
        Err(e) => problems.push(e.to_string()),
    }
    if problems.is_empty() {
        let pack = if snapshot.search.occupations.is_empty() {
            "no occupation packs selected".to_owned()
        } else {
            format!("occupations: {}", snapshot.search.occupations.join(", "))
        };
        check("search profile", CheckStatus::Ok, pack)
    } else {
        check("search profile", CheckStatus::Warn, problems.join("\n"))
    }
}

fn data_dir(ctx: &Context) -> DoctorCheck {
    let dir = ctx.paths.data_dir();
    let probe = dir.join(".doctor-write-test");
    let result = ctx
        .paths
        .ensure_data_dir()
        .map_err(|e| e.to_string())
        .and_then(|()| {
            std::fs::write(&probe, b"ok")
                .and_then(|()| std::fs::remove_file(&probe))
                .map_err(|e| format!("{}: {e}", dir.display()))
        });
    match result {
        // Gantry restricts only a directory it creates; one the user
        // pointed it at keeps its permissions.
        Ok(()) if gantry_platform::readable_by_others(dir) => check(
            "data directory",
            CheckStatus::Warn,
            format!(
                "{} is writable, but other users can access it; run `chmod 700` on it",
                dir.display()
            ),
        ),
        Ok(()) => check(
            "data directory",
            CheckStatus::Ok,
            format!("{} is writable", dir.display()),
        ),
        Err(e) => check("data directory", CheckStatus::Fail, e),
    }
}

fn database(ctx: &Context) -> DoctorCheck {
    let path = ctx.paths.db_path();
    let store = match ctx
        .paths
        .ensure_data_dir()
        .map_err(|e| e.to_string())
        .and_then(|()| Store::open(&path).map_err(|e| e.to_string()))
    {
        Ok(s) => s,
        Err(e) => return check("database", CheckStatus::Fail, e),
    };
    let integrity = store.integrity_check().map_err(|e| e.to_string());
    let mode = store.journal_mode().map_err(|e| e.to_string());
    let version = store.schema_version().map_err(|e| e.to_string());
    match (integrity, mode, version) {
        (Ok(i), Ok(mode), Ok(v)) if i == ["ok"] && mode == "wal" => check(
            "database",
            CheckStatus::Ok,
            format!("{}: schema {v}, WAL, integrity ok", path.display()),
        ),
        (Ok(i), Ok(mode), Ok(v)) => check(
            "database",
            CheckStatus::Fail,
            format!(
                "{}: schema {v}, journal mode {mode}, integrity: {}",
                path.display(),
                i.join("; ")
            ),
        ),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => check("database", CheckStatus::Fail, e),
    }
}

/// Looks up a key that never exists; success means the store answered.
fn secret_store(services: &Services) -> DoctorCheck {
    const NAME: &str = "secret store";
    let result = (services.secrets)().and_then(|s| s.get("doctor-probe"));
    match result {
        Ok(_) => check(NAME, CheckStatus::Ok, "OS keyring answered"),
        Err(PlatformError::Unsupported(what)) => check(
            NAME,
            CheckStatus::Skip,
            format!("{what} is not ported to this OS yet"),
        ),
        Err(e) => check(
            NAME,
            CheckStatus::Warn,
            format!("{e}; API keys (M3) need a running keyring such as GNOME Keyring or KWallet"),
        ),
    }
}

fn browser(services: &Services) -> DoctorCheck {
    const NAME: &str = "browser";
    match services.browser.locate() {
        Ok(Some(b)) => check(
            NAME,
            CheckStatus::Ok,
            format!("{} at {}", b.name, b.path.display()),
        ),
        Ok(None) => check(
            NAME,
            CheckStatus::Warn,
            "no Chromium, Chrome, Brave or Edge found in PATH; the review queue (M6) needs one",
        ),
        Err(PlatformError::Unsupported(what)) => check(
            NAME,
            CheckStatus::Skip,
            format!("{what} is not ported to this OS yet"),
        ),
        Err(e) => check(NAME, CheckStatus::Warn, e.to_string()),
    }
}

fn scheduler(services: &Services) -> DoctorCheck {
    const NAME: &str = "scheduler";
    match services.scheduler.status() {
        Ok(ScheduleStatus::NotInstalled) => check(
            NAME,
            CheckStatus::Warn,
            "no nightly run is scheduled; scheduling arrives in M7 (run `gantry run --discover-only` by hand until then)",
        ),
        Ok(ScheduleStatus::Installed {
            enabled: true,
            active: true,
            detail,
        }) => check(NAME, CheckStatus::Ok, detail),
        Ok(ScheduleStatus::Installed { detail, .. }) => check(NAME, CheckStatus::Warn, detail),
        Ok(ScheduleStatus::Unavailable(detail)) => check(NAME, CheckStatus::Warn, detail),
        Err(PlatformError::Unsupported(what)) => check(
            NAME,
            CheckStatus::Skip,
            format!("{what} is not ported to this OS yet"),
        ),
        Err(e) => check(NAME, CheckStatus::Warn, e.to_string()),
    }
}
