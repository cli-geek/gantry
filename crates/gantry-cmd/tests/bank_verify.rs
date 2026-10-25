//! `bank_verify`, `bank_pending`, `bank_verify_all` and `bank_coverage`
//! against a temporary config directory.
//! Golden files: `GANTRY_UPDATE_GOLDEN=1 cargo test -p gantry-cmd --test bank_verify`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use gantry_cmd::{
    BankCoverage, CmdError, Context, ExitCode, Paths, PendingItem, SetAction, VerifyOutcome,
    bank_coverage, bank_list, bank_pending, bank_set, bank_verify, bank_verify_all,
};
use gantry_core::questions::AnswerState;
use gantry_store::{bundled_catalog, load_snapshot};
use serde_json::json;

const CITIZEN: &str = "legal.authorization.us_citizen";

struct Fixture {
    ctx: Context,
    config: PathBuf,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        let ctx = Context::new(Paths::new(&config, dir.path().join("data")));
        Self {
            ctx,
            config,
            _dir: dir,
        }
    }

    fn write(&self, file: &str, text: &str) {
        std::fs::write(self.config.join(file), text).unwrap();
    }

    fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.config.join(file)).unwrap()
    }

    fn table(&self, file: &str) -> toml::Table {
        self.read(file).parse().unwrap()
    }

    fn set(&self, id: &str, text: &str, unverified: bool) {
        bank_set(
            &self.ctx,
            id,
            SetAction::Value(text.to_owned()),
            unverified,
            false,
        )
        .unwrap();
    }

    fn set_json(&self, id: &str, value: serde_json::Value, unverified: bool) {
        bank_set(
            &self.ctx,
            id,
            SetAction::ValueJson(value),
            unverified,
            false,
        )
        .unwrap();
    }

    fn verify(&self, ids: &[&str]) -> Result<VerifyOutcome, CmdError> {
        let ids: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
        bank_verify(&self.ctx, &ids, false)
    }

    fn state(&self, id: &str) -> AnswerState {
        bank_list(&self.ctx, &Default::default(), true)
            .unwrap()
            .rows
            .into_iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no row {id}"))
            .state
    }

    fn pending_ids(&self) -> Vec<String> {
        bank_pending(&self.ctx, false)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect()
    }
}

fn golden(name: &str, actual: &serde_json::Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let text = serde_json::to_string_pretty(actual).unwrap() + "\n";
    if std::env::var_os("GANTRY_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    } else {
        let want = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            want, text,
            "{name} differs; run GANTRY_UPDATE_GOLDEN=1 cargo test -p gantry-cmd --test bank_verify"
        );
    }
}

fn exit(err: &CmdError) -> ExitCode {
    err.exit_code()
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn verify_flips_an_unverified_value() {
    let f = Fixture::new();
    f.set("profile.contact.email", "ada@example.com", true);
    assert_eq!(f.state("profile.contact.email"), AnswerState::Suggested);
    let before = f.table("profile.toml");
    let item = before["contact"]["email"].as_table().unwrap().clone();
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let out = f.verify(&["profile.contact.email"]).unwrap();
    assert_eq!(out.verified, strs(&["profile.contact.email"]));
    assert!(out.already.is_empty() && out.unknown.is_empty());
    assert_eq!(f.state("profile.contact.email"), AnswerState::Answered);

    let after = f.table("profile.toml");
    let now = after["contact"]["email"].as_table().unwrap();
    assert_eq!(now["verified"].as_bool(), Some(true));
    assert_eq!(now["value"], item["value"]);
    assert_eq!(now["created_at"], item["created_at"]);
    assert_ne!(now["updated_at"], item["updated_at"]);
}

#[test]
fn us_citizen_flow_pending_order_and_verify() {
    let f = Fixture::new();
    f.set(CITIZEN, "yes", false);
    let pending = bank_pending(&f.ctx, false).unwrap();
    let ids: Vec<&str> = pending.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "legal.authorization.us_permanent_resident",
            "legal.authorization.authorized_us",
            "legal.authorization.require_sponsorship_now",
            "legal.authorization.require_sponsorship_future",
            "legal.authorization.us_person",
        ]
    );
    for p in &pending {
        assert_eq!(
            p.source.as_deref(),
            Some("derived:legal.authorization.us_citizen")
        );
    }

    let out = f.verify(&["legal.authorization.authorized_us"]).unwrap();
    assert_eq!(out.verified, strs(&["legal.authorization.authorized_us"]));
    let legal = f.table("legal.toml");
    let item = legal["authorization"]["authorized_us"].as_table().unwrap();
    assert_eq!(item["verified"].as_bool(), Some(true));
    assert_eq!(
        item["source"].as_str(),
        Some("derived:legal.authorization.us_citizen")
    );

    let rest: Vec<String> = f.pending_ids();
    assert_eq!(rest.len(), 4);
    f.verify(&rest.iter().map(String::as_str).collect::<Vec<_>>())
        .unwrap();
    assert!(f.pending_ids().is_empty());
    let banks = load_snapshot(&f.config).unwrap().banks;
    assert_eq!(banks.needs_sponsorship(), Some(false));
}

#[test]
fn verify_triggers_derive() {
    let f = Fixture::new();
    f.set("profile.education.highest_level", "bachelors", true);
    assert_eq!(
        f.state("profile.education.has_bachelors"),
        AnswerState::Missing
    );
    let out = f.verify(&["profile.education.highest_level"]).unwrap();
    let derived: Vec<&str> = out.derived.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(derived, ["profile.education.has_bachelors"]);
    assert_eq!(out.derived[0].from, "profile.education.highest_level");
    assert_eq!(out.derived[0].value, Some(json!(true)));
    assert_eq!(
        f.state("profile.education.has_bachelors"),
        AnswerState::Suggested
    );
}

#[test]
fn list_question_verifies_every_entry() {
    let f = Fixture::new();
    f.set_json(
        "profile.education.history",
        json!([
            { "institution": "PE", "credential": "NCEES" },
            { "institution": "CPA", "credential": "AICPA" }
        ]),
        true,
    );
    assert_eq!(f.state("profile.education.history"), AnswerState::Suggested);
    let out = f.verify(&["profile.education.history"]).unwrap();
    assert_eq!(out.verified, strs(&["profile.education.history"]));
    let t = f.table("profile.toml");
    let rows = t["education"]["history"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r["verified"].as_bool() == Some(true)));
    assert_eq!(f.state("profile.education.history"), AnswerState::Answered);

    let again = f.verify(&["profile.education.history"]).unwrap();
    assert_eq!(again.already, strs(&["profile.education.history"]));
}

#[test]
fn mixed_and_all_unknown_batches() {
    let f = Fixture::new();
    f.set("profile.contact.email", "ada@example.com", true);
    let out = f.verify(&["no.such.id", "profile.contact.email"]).unwrap();
    assert_eq!(out.unknown, strs(&["no.such.id"]));
    assert_eq!(out.verified, strs(&["profile.contact.email"]));

    let err = f.verify(&["no.such.id", "also.not.one"]).unwrap_err();
    assert!(matches!(err, CmdError::InvalidInput(_)), "{err}");
    assert_eq!(exit(&err), ExitCode::Usage);

    // Attestation or prohibited questions have no address.
    let catalog = bundled_catalog().unwrap();
    if let Some(def) = catalog.iter().find(|d| !d.is_addressable()) {
        let out = f
            .verify(&[def.id.as_str(), "profile.contact.email"])
            .unwrap();
        assert_eq!(out.unknown, strs(&[def.id.as_str()]));
    }
}

#[test]
fn already_and_nothing_stored_buckets() {
    let f = Fixture::new();
    f.set("profile.contact.email", "ada@example.com", false);
    let out = f
        .verify(&["profile.contact.email", "profile.contact.phone"])
        .unwrap();
    assert_eq!(out.already, strs(&["profile.contact.email"]));
    assert_eq!(out.nothing_stored, strs(&["profile.contact.phone"]));
    assert!(out.verified.is_empty() && out.derived.is_empty());
}

#[test]
fn verify_all_needs_confirmation() {
    let f = Fixture::new();
    f.set(CITIZEN, "yes", false);
    let before = f.read("legal.toml");
    let err = bank_verify_all(&f.ctx, false, false).unwrap_err();
    assert_eq!(exit(&err), ExitCode::Usage);
    assert_eq!(f.read("legal.toml"), before);
    assert_eq!(f.pending_ids().len(), 5);

    let out = bank_verify_all(&f.ctx, true, false).unwrap();
    assert_eq!(out.verified.len(), 5);
    assert!(f.pending_ids().is_empty());
}

#[test]
fn pending_and_derived_values_are_redacted() {
    let f = Fixture::new();
    let out = bank_set(
        &f.ctx,
        CITIZEN,
        SetAction::Value("yes".into()),
        false,
        false,
    )
    .unwrap();
    assert!(out.derived.iter().all(|d| d.redacted && d.value.is_none()));

    let hidden = bank_pending(&f.ctx, false).unwrap();
    assert_eq!(hidden.len(), 5);
    assert!(hidden.iter().all(|p| p.redacted && p.value.is_none()));
    let shown = bank_pending(&f.ctx, true).unwrap();
    assert!(shown.iter().all(|p| !p.redacted && p.value.is_some()));

    // Availability is hidden too; a verify reports its derived rows redacted.
    f.set("availability.hours.desired_per_week", "30", true);
    let hidden = bank_pending(&f.ctx, false).unwrap();
    let hours = hidden
        .iter()
        .find(|p| p.id == "availability.hours.desired_per_week")
        .unwrap();
    assert!(hours.redacted && hours.value.is_none());
    f.set("profile.contact.email", "ada@example.com", true);
    let email: PendingItem = bank_pending(&f.ctx, false)
        .unwrap()
        .into_iter()
        .find(|p| p.id == "profile.contact.email")
        .unwrap();
    assert!(!email.redacted);
    assert_eq!(email.value, Some(json!("ada@example.com")));
    assert_eq!(email.source, None);
}

#[test]
fn coverage_meter() {
    let f = Fixture::new();
    let catalog = bundled_catalog().unwrap();
    let declined = catalog
        .iter()
        .filter(|d| d.is_addressable() && !d.optional && catalog.pack_of(&d.id).is_none())
        .filter(|d| d.default_policy == Some(gantry_core::questions::PolicyMode::Decline))
        .count();
    let empty = bank_coverage(&f.ctx).unwrap();
    assert!(declined > 0);
    assert_eq!(empty.report.overall.covered as usize, declined);
    assert_eq!(empty.report.overall.answered, 0);
    let e = &empty.report.sections[&gantry_core::questions::Section::E];
    assert_eq!(e.covered as usize, declined, "only E declines by default");

    assert!(
        empty
            .always_ask
            .iter()
            .any(|r| r.id == "legal.criminal.convicted")
    );
    let ids: Vec<&str> = empty.always_ask.iter().map(|r| r.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted);

    // email: weight 2, non-optional, not always-ask.
    f.set("profile.contact.email", "ada@example.com", false);
    let after = bank_coverage(&f.ctx).unwrap();
    let (o0, o1) = (&empty.report.overall, &after.report.overall);
    assert_eq!(o1.answered, 1);
    assert_eq!(o1.weighted_denominator, o0.weighted_denominator);
    assert_eq!(o1.weighted_covered, o0.weighted_covered + 2);
    let want = (o0.weighted_covered + 2) as f64 / o0.weighted_denominator as f64;
    assert_eq!(o1.weighted_ratio, Some(want));
    assert!(o1.weighted_ratio > o0.weighted_ratio);
}

#[test]
fn composable_after_a_source_is_verified() {
    let f = Fixture::new();
    assert_eq!(f.state("motivation.why_company"), AnswerState::Missing);
    let before = bank_coverage(&f.ctx).unwrap().report.overall.composable;
    f.set_json(
        "motivation.domains",
        json!([{ "domain": "robotics", "why": "hands-on" }]),
        true,
    );
    // Unverified sources do not count.
    assert_eq!(f.state("motivation.why_company"), AnswerState::Missing);
    f.verify(&["motivation.domains"]).unwrap();
    assert_eq!(f.state("motivation.why_company"), AnswerState::Composable);
    let after = bank_coverage(&f.ctx).unwrap().report.overall;
    assert!(after.composable > before);
}

#[test]
fn pack_questions_need_the_occupation() {
    let f = Fixture::new();
    let core = bank_coverage(&f.ctx).unwrap();
    assert!(!core.report.packs.contains_key("technology"));
    f.write(
        "search.toml",
        "schema_version = 1\noccupations = [\"technology\"]\n",
    );
    let tech = bank_coverage(&f.ctx).unwrap();
    assert!(tech.report.packs.contains_key("technology"));
    assert!(tech.report.overall.denominator > core.report.overall.denominator);
}

#[test]
fn unparsable_bank_file_is_a_config_error_and_stays_untouched() {
    let f = Fixture::new();
    f.write("legal.toml", "not [ toml");
    let ids = strs(&[CITIZEN]);
    assert_eq!(
        exit(&bank_verify(&f.ctx, &ids, false).unwrap_err()),
        ExitCode::Config
    );
    assert_eq!(
        exit(&bank_pending(&f.ctx, false).unwrap_err()),
        ExitCode::Config
    );
    assert_eq!(
        exit(&bank_verify_all(&f.ctx, true, false).unwrap_err()),
        ExitCode::Config
    );
    assert_eq!(exit(&bank_coverage(&f.ctx).unwrap_err()), ExitCode::Config);
    assert_eq!(f.read("legal.toml"), "not [ toml");
}

#[test]
fn golden_coverage() {
    let f = Fixture::new();
    f.set("profile.contact.email", "ada@example.com", false);
    f.set("profile.name.legal_first", "Ada", false);
    f.set("profile.education.highest_level", "bachelors", true);
    f.set_json(
        "motivation.domains",
        json!([{ "domain": "robotics", "why": "hands-on" }]),
        false,
    );
    bank_set(&f.ctx, "eeo.gender", SetAction::Decline, false, false).unwrap();
    let cov: BankCoverage = bank_coverage(&f.ctx).unwrap();
    golden("bank-coverage.json", &serde_json::to_value(&cov).unwrap());
}

#[test]
fn golden_verify() {
    let f = Fixture::new();
    f.set(CITIZEN, "yes", false);
    f.set("profile.education.highest_level", "bachelors", true);
    f.set("profile.contact.email", "nobody", true);
    let ids = strs(&[
        "legal.authorization.authorized_us",
        "profile.education.highest_level",
        "profile.contact.email",
        "profile.contact.phone",
        CITIZEN,
        "no.such.id",
    ]);
    let out = bank_verify(&f.ctx, &ids, true).unwrap();
    golden("bank-verify.json", &serde_json::to_value(&out).unwrap());
}
