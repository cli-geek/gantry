//! `bank_list` and `bank_set` against a temporary config directory.
//! Golden files: `GANTRY_UPDATE_GOLDEN=1 cargo test -p gantry-cmd --test bank`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use gantry_cmd::{
    BankFilter, BankList, CmdError, Context, ExitCode, Paths, SetAction, SetOutcome, bank_list,
    bank_set,
};
use gantry_core::questions::{AnswerState, Sensitivity};
use gantry_store::load_snapshot;
use serde_json::json;

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

    fn set(&self, id: &str, text: &str) -> SetOutcome {
        bank_set(
            &self.ctx,
            id,
            SetAction::Value(text.to_owned()),
            false,
            false,
        )
        .unwrap()
    }

    fn set_with(&self, id: &str, action: SetAction) -> Result<SetOutcome, CmdError> {
        bank_set(&self.ctx, id, action, false, false)
    }

    fn list(&self, values: bool) -> BankList {
        bank_list(&self.ctx, &BankFilter::default(), values).unwrap()
    }

    fn row(&self, id: &str, values: bool) -> gantry_cmd::BankRow {
        self.list(values)
            .rows
            .into_iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no row {id}"))
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
            "{name} differs; run GANTRY_UPDATE_GOLDEN=1 cargo test -p gantry-cmd --test bank"
        );
    }
}

fn exit(err: &CmdError) -> ExitCode {
    err.exit_code()
}

#[test]
fn golden_set_and_list() {
    let f = Fixture::new();
    // With values shown, so the golden records what derive suggests.
    let first = bank_set(
        &f.ctx,
        "legal.authorization.us_citizen",
        SetAction::Value("yes".into()),
        false,
        true,
    )
    .unwrap();
    golden("bank-set.json", &serde_json::to_value(&first).unwrap());

    f.set("profile.contact.email", "ada@example.com");
    f.set("eeo.gender", "Female");
    f.set("availability.hours.desired_per_week", "30");
    bank_set(
        &f.ctx,
        "legal.authorization.work_permit_end",
        SetAction::Value("2027-06-30".into()),
        true,
        false,
    )
    .unwrap();
    f.set_with(
        "profile.education.history",
        SetAction::ValueJson(json!([
            { "institution": "PE", "credential": "NCEES" },
            { "institution": "CPA", "credential": "AICPA" }
        ])),
    )
    .unwrap();
    f.set_with("legal.age.over_18", SetAction::Decline).unwrap();

    let mut list = f.list(true);
    list.rows.retain(|r| r.state != AnswerState::Missing);
    golden("bank-list.json", &serde_json::to_value(&list).unwrap());
}

#[test]
fn us_citizen_derives_unverified_suggestions() {
    let f = Fixture::new();
    let out = f.set("legal.authorization.us_citizen", "yes");
    let derived: Vec<_> = out
        .derived
        .iter()
        .map(|d| (d.id.as_str(), d.from.as_str()))
        .collect();
    let from = "legal.authorization.us_citizen";
    assert_eq!(
        derived,
        vec![
            ("legal.authorization.authorized_us", from),
            ("legal.authorization.require_sponsorship_future", from),
            ("legal.authorization.require_sponsorship_now", from),
            ("legal.authorization.us_permanent_resident", from),
            ("legal.authorization.us_person", from),
        ]
    );
    let values = f.set_values_of(&out);
    assert_eq!(
        values,
        vec![true, false, false, false, true],
        "per intake §4"
    );

    let legal = f.table("legal.toml");
    for name in [
        "authorized_us",
        "require_sponsorship_now",
        "require_sponsorship_future",
        "us_person",
        "us_permanent_resident",
    ] {
        let item = legal["authorization"][name].as_table().unwrap();
        assert_eq!(item["verified"].as_bool(), Some(false), "{name}");
        assert_eq!(
            item["source"].as_str(),
            Some("derived:legal.authorization.us_citizen")
        );
        assert_eq!(
            item["id"].as_str(),
            Some(format!("legal.authorization.{name}").as_str())
        );
    }
    let citizen = legal["authorization"]["us_citizen"].as_table().unwrap();
    assert_eq!(citizen["verified"].as_bool(), Some(true));
    assert!(!citizen.contains_key("source"));
    assert_eq!(
        f.row("legal.authorization.authorized_us", true).state,
        AnswerState::Suggested
    );
}

impl Fixture {
    /// The bool values stored for each derived row, in order.
    fn set_values_of(&self, out: &SetOutcome) -> Vec<bool> {
        out.derived
            .iter()
            .map(|d| self.row(&d.id, true).value.unwrap().as_bool().unwrap())
            .collect()
    }
}

#[test]
fn derive_never_overwrites() {
    let f = Fixture::new();
    let out = f.set("legal.authorization.us_permanent_resident", "yes");
    let ids: Vec<_> = out.derived.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "legal.authorization.authorized_us",
            "legal.authorization.require_sponsorship_future",
            "legal.authorization.require_sponsorship_now",
            "legal.authorization.us_person",
        ]
    );
    // The permanent-resident answer is occupied, so a citizen answer cannot
    // change it, and the four suggestions already there stay as they are.
    let out = f.set("legal.authorization.us_citizen", "yes");
    assert!(out.derived.is_empty(), "{:?}", out.derived);
    let row = f.row("legal.authorization.us_permanent_resident", true);
    assert_eq!(row.value, Some(json!(true)));
    assert_eq!(row.verified, Some(true));
}

#[test]
fn unverified_set_derives_nothing() {
    let f = Fixture::new();
    let out = bank_set(
        &f.ctx,
        "legal.authorization.us_citizen",
        SetAction::Value("yes".into()),
        true,
        false,
    )
    .unwrap();
    assert!(!out.verified);
    assert!(out.derived.is_empty());
    assert!(
        f.table("legal.toml")["authorization"]
            .as_table()
            .unwrap()
            .get("authorized_us")
            .is_none()
    );
}

#[test]
fn snapshot_sees_verified_values_only() {
    let f = Fixture::new();
    f.set("legal.authorization.us_citizen", "yes");
    let banks = load_snapshot(&f.config).unwrap().banks;
    assert_eq!(banks.us_citizen(), Some(true));
    assert_eq!(
        banks.needs_sponsorship(),
        None,
        "derived values are unverified"
    );
    f.set("legal.authorization.require_sponsorship_now", "no");
    f.set("legal.authorization.require_sponsorship_future", "no");
    let banks = load_snapshot(&f.config).unwrap().banks;
    assert_eq!(banks.needs_sponsorship(), Some(false));
}

#[test]
fn redaction_follows_sensitivity() {
    let f = Fixture::new();
    f.set("legal.authorization.us_citizen", "yes");
    f.set("eeo.gender", "female");
    f.set("legal.clearance.level", "TS/SCI");
    f.set("availability.hours.desired_per_week", "30");
    f.set("profile.contact.email", "ada@example.com");

    for id in [
        "legal.authorization.us_citizen",
        "eeo.gender",
        "legal.clearance.level",
        "availability.hours.desired_per_week",
    ] {
        let hidden = f.row(id, false);
        assert!(hidden.redacted, "{id}");
        assert_eq!(hidden.value, None, "{id}");
        assert_eq!(hidden.verified, Some(true), "{id}");
        let shown = f.row(id, true);
        assert!(!shown.redacted, "{id}");
        assert!(shown.value.is_some(), "{id}");
    }
    assert_eq!(f.row("eeo.gender", true).value, Some(json!("female")));
    assert_eq!(
        f.row("legal.clearance.level", true).value,
        Some(json!("ts_sci"))
    );
    let email = f.row("profile.contact.email", false);
    assert!(!email.redacted);
    assert_eq!(email.value, Some(json!("ada@example.com")));
    // Nothing stored, nothing to redact.
    let empty = f.row("legal.age.over_18", false);
    assert!(!empty.redacted);
    assert_eq!(empty.value, None);
    assert_eq!(empty.verified, None);

    let list = f.list(false);
    assert!(list.rows.iter().filter(|r| r.redacted).all(|r| matches!(
        r.sensitivity,
        Sensitivity::Legal | Sensitivity::Eeo | Sensitivity::Clearance | Sensitivity::Availability
    )));

    // `bank_set` output follows the same rule, derived rows included.
    let g = Fixture::new();
    let hidden = g.set("legal.authorization.us_citizen", "yes");
    assert!(hidden.redacted && hidden.value.is_none());
    assert!(
        hidden
            .derived
            .iter()
            .all(|d| d.redacted && d.value.is_none())
    );
    let h = Fixture::new();
    let shown = bank_set(
        &h.ctx,
        "legal.authorization.us_citizen",
        SetAction::Value("yes".into()),
        false,
        true,
    )
    .unwrap();
    assert_eq!(shown.value, Some(json!(true)));
    assert!(
        shown
            .derived
            .iter()
            .all(|d| !d.redacted && d.value.is_some())
    );
    let contact = h.set("profile.contact.email", "a@b.co");
    assert_eq!(contact.value, Some(json!("a@b.co")));
}

#[test]
fn unknown_keys_survive_and_unknown_paths_warn() {
    let f = Fixture::new();
    f.write(
        "legal.toml",
        "autofill = true\n\
         [future.thing]\nverified = true\nextra = 'x'\n\
         [authorization.mystery]\nid = 'm'\nvalue = 1\nverified = false\n\
         [authorization.us_citizen]\nid = 'opaque-id'\nvalue = false\nverified = false\n\
         created_at = '2026-01-01T00:00:00Z'\nupdated_at = '2026-01-01T00:00:00Z'\nnote = 'keep'\n",
    );
    let before = f.list(true);
    assert!(
        before
            .warnings
            .iter()
            .any(|w| w.contains("legal.authorization.mystery"))
    );
    assert!(
        before
            .warnings
            .iter()
            .any(|w| w.contains("legal.future.thing"))
    );

    f.set("legal.authorization.us_citizen", "yes");
    let legal = f.table("legal.toml");
    assert_eq!(legal["autofill"].as_bool(), Some(true));
    assert_eq!(legal["future"]["thing"]["extra"].as_str(), Some("x"));
    assert_eq!(
        legal["authorization"]["mystery"]["value"].as_integer(),
        Some(1)
    );
    let item = legal["authorization"]["us_citizen"].as_table().unwrap();
    assert_eq!(item["id"].as_str(), Some("opaque-id"));
    assert_eq!(item["note"].as_str(), Some("keep"));
    assert_eq!(item["created_at"].as_str(), Some("2026-01-01T00:00:00Z"));
    assert_ne!(item["updated_at"].as_str(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(item["value"].as_bool(), Some(true));
    assert_eq!(item["verified"].as_bool(), Some(true));
    assert!(
        f.list(true)
            .warnings
            .iter()
            .any(|w| w.contains("legal.authorization.mystery"))
    );
}

#[test]
fn value_clears_policy_and_policy_keeps_value() {
    let f = Fixture::new();
    f.set("eeo.gender", "female");
    assert_eq!(f.row("eeo.gender", true).state, AnswerState::Answered);

    let out = f.set_with("eeo.gender", SetAction::AlwaysAsk).unwrap();
    assert_eq!(out.action, "always_ask");
    assert!(out.verified && !out.policy_cleared);
    let row = f.row("eeo.gender", true);
    assert_eq!(row.state, AnswerState::AlwaysAsk);
    assert_eq!(row.value, Some(json!("female")), "the stored value stays");
    let answers = f.table("answers.toml");
    let policy = answers["policy"]["eeo.gender"].as_table().unwrap();
    assert_eq!(policy["value"].as_str(), Some("always_ask"));
    assert_eq!(policy["verified"].as_bool(), Some(true));
    assert_eq!(policy["id"].as_str(), Some("eeo.gender"));

    let out = f.set_with("eeo.gender", SetAction::Decline).unwrap();
    assert_eq!(out.action, "decline");
    assert_eq!(f.row("eeo.gender", false).state, AnswerState::Declined);

    let out = f.set("eeo.gender", "male");
    assert!(out.policy_cleared);
    assert_eq!(f.row("eeo.gender", true).state, AnswerState::Answered);
    assert!(
        f.table("answers.toml")["policy"]
            .as_table()
            .unwrap()
            .is_empty()
    );

    f.set_with("eeo.gender", SetAction::NotApplicable).unwrap();
    assert_eq!(f.row("eeo.gender", true).state, AnswerState::NotApplicable);
    let out = f.set_with("eeo.gender", SetAction::ClearPolicy).unwrap();
    assert!(out.policy_cleared);
    assert_eq!(out.action, "clear_policy");
    assert_eq!(f.row("eeo.gender", true).state, AnswerState::Answered);
    let out = f.set_with("eeo.gender", SetAction::ClearPolicy).unwrap();
    assert!(!out.policy_cleared);
}

#[test]
fn clearing_or_setting_values_does_not_create_answers_toml() {
    let f = Fixture::new();
    f.set("profile.contact.email", "a@b.co");
    f.set_with("profile.contact.email", SetAction::ClearPolicy)
        .unwrap();
    assert!(!f.config.join("answers.toml").exists());
}

#[test]
fn rejected_input_is_a_usage_error_and_changes_nothing() {
    let f = Fixture::new();
    let cases: Vec<(&str, SetAction)> = vec![
        ("legal.nope.nothing", SetAction::Value("x".into())),
        ("legal.nope.nothing", SetAction::Decline),
        ("legal.clearance.level", SetAction::Value("galactic".into())),
        (
            "legal.authorization.us_citizen",
            SetAction::Value("maybe".into()),
        ),
        (
            "legal.authorization.work_permit_end",
            SetAction::Value("2027-02-30".into()),
        ),
        (
            "availability.hours.desired_per_week",
            SetAction::Value("lots".into()),
        ),
        ("profile.education.history", SetAction::Value("PE".into())),
        (
            "profile.education.history",
            SetAction::ValueJson(json!("PE")),
        ),
        (
            "legal.authorization.us_citizen",
            SetAction::ValueJson(json!(null)),
        ),
        (
            "legal.authorization.us_citizen",
            SetAction::ValueJson(json!("yes")),
        ),
    ];
    for (id, action) in cases {
        let err = f.set_with(id, action.clone()).unwrap_err();
        assert!(
            matches!(err, CmdError::InvalidInput(_)),
            "{id} {action:?}: {err}"
        );
        assert_eq!(exit(&err), ExitCode::Usage);
    }
    for file in [
        "legal.toml",
        "availability.toml",
        "profile.toml",
        "answers.toml",
    ] {
        assert!(!f.config.join(file).exists(), "{file}");
    }
}

#[test]
fn enum_and_bool_accept_labels() {
    let f = Fixture::new();
    f.set("legal.clearance.level", "Top Secret");
    assert_eq!(
        f.row("legal.clearance.level", true).value,
        Some(json!("top_secret"))
    );
    f.set("legal.authorization.us_citizen", "FALSE");
    assert_eq!(
        f.row("legal.authorization.us_citizen", true).value,
        Some(json!(false))
    );
    f.set("availability.hours.desired_per_week", "37.5");
    assert_eq!(
        f.row("availability.hours.desired_per_week", true).value,
        Some(json!(37.5))
    );
}

#[test]
fn unparsable_bank_file_is_a_config_error_and_stays_untouched() {
    let f = Fixture::new();
    f.write("legal.toml", "not [ toml");
    let err = f.set_with(
        "legal.authorization.us_citizen",
        SetAction::Value("yes".into()),
    );
    let err = err.unwrap_err();
    assert!(matches!(err, CmdError::Config(_)), "{err}");
    assert_eq!(exit(&err), ExitCode::Config);
    assert_eq!(f.read("legal.toml"), "not [ toml");

    // Another file's address is refused too: the banks are read as a set.
    let err = f.set_with("profile.contact.email", SetAction::Value("a@b.co".into()));
    assert_eq!(exit(&err.unwrap_err()), ExitCode::Config);
    assert!(!f.config.join("profile.toml").exists());

    let err = bank_list(&f.ctx, &BankFilter::default(), false).unwrap_err();
    assert_eq!(exit(&err), ExitCode::Config);
}

#[test]
fn list_set_replaces_entries() {
    let f = Fixture::new();
    f.set_with(
        "profile.education.history",
        SetAction::ValueJson(json!([
            { "institution": "PE", "credential": "NCEES" },
            { "institution": "CPA", "credential": "AICPA" },
            { "institution": "RN", "credential": "NCSBN" }
        ])),
    )
    .unwrap();
    let ids = |t: &toml::Table| -> Vec<String> {
        t["education"]["history"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        ids(&f.table("profile.toml")),
        ["history-1", "history-2", "history-3"]
    );
    let row = f.row("profile.education.history", true);
    assert_eq!(row.state, AnswerState::Answered);
    assert_eq!(row.verified, Some(true));

    f.set_with(
        "profile.education.history",
        SetAction::ValueJson(json!([{ "institution": "PE", "credential": "NCEES" }])),
    )
    .unwrap();
    let t = f.table("profile.toml");
    assert_eq!(ids(&t), ["history-1"]);
    assert_eq!(
        t["education"]["history"][0]["value"]["institution"].as_str(),
        Some("PE")
    );
    assert_eq!(
        f.row("profile.education.history", true).value,
        Some(json!([{ "institution": "PE", "credential": "NCEES" }]))
    );

    // A list of plain strings.
    f.set_with(
        "legal.authorization.citizenship_countries",
        SetAction::ValueJson(json!(["US", "CA"])),
    )
    .unwrap();
    assert_eq!(
        f.row("legal.authorization.citizenship_countries", true)
            .value,
        Some(json!(["US", "CA"]))
    );
}

#[test]
fn filters_and_applicability() {
    let f = Fixture::new();
    let all = f.list(false);
    assert!(all.rows.iter().all(|r| r.pack.is_none()), "no occupations");
    assert!(all.rows.iter().any(|r| r.state == AnswerState::Declined));
    f.write(
        "search.toml",
        "schema_version = 1\noccupations = [\"technology\"]\n",
    );
    let with_pack = f.list(false);
    assert!(with_pack.rows.len() > all.rows.len());
    assert!(
        with_pack
            .rows
            .iter()
            .any(|r| r.pack.as_deref() == Some("technology"))
    );

    f.set("eeo.gender", "female");
    let only = bank_list(
        &f.ctx,
        &BankFilter {
            state: Some(AnswerState::Answered),
            ..BankFilter::default()
        },
        false,
    )
    .unwrap();
    assert_eq!(only.rows.len(), 1);
    assert_eq!(only.rows[0].id, "eeo.gender");

    let pack = bank_list(
        &f.ctx,
        &BankFilter {
            pack: Some("technology".into()),
            sensitivity: Some(Sensitivity::FreeText),
            ..BankFilter::default()
        },
        false,
    )
    .unwrap();
    assert!(!pack.rows.is_empty());
    assert!(
        pack.rows
            .iter()
            .all(|r| r.pack.as_deref() == Some("technology"))
    );
    let core = bank_list(
        &f.ctx,
        &BankFilter {
            pack: Some("core".into()),
            ..BankFilter::default()
        },
        false,
    )
    .unwrap();
    assert_eq!(core.rows.len(), all.rows.len());
}
