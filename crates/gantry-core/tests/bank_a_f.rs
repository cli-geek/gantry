//! The core bank sections A to F: intake set, derive rules and default policies.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use gantry_core::questions::{Catalog, PolicyMode, Section, Sensitivity, derive};
use toml::Value;

/// The intake set of docs/design/m2a-intake.md §3, ranks 1 to 22.
const INTAKE: [&str; 22] = [
    "profile.name.legal_first",
    "profile.name.legal_last",
    "profile.name.preferred",
    "profile.contact.email",
    "profile.contact.phone",
    "profile.address.city",
    "profile.address.region",
    "profile.address.country",
    "profile.links.linkedin",
    "profile.links.website",
    "profile.referral.default_source",
    "legal.authorization.us_citizen",
    "legal.authorization.us_permanent_resident",
    "legal.authorization.authorized_us",
    "legal.authorization.require_sponsorship_now",
    "legal.authorization.require_sponsorship_future",
    "legal.age.over_18",
    "availability.start.earliest_date",
    "availability.relocation.willing",
    "availability.work_mode",
    "availability.pay.desired_annual",
    "profile.education.highest_level",
];

const AUTH: &str = "legal.authorization";

fn in_a_to_f(id: &str) -> bool {
    ["profile.", "legal.", "eeo.", "availability."]
        .iter()
        .any(|p| id.starts_with(p))
}

/// Suggestions after the user verifies `from = value`, by target id.
fn suggest(catalog: &Catalog, from: &str, value: Value) -> BTreeMap<String, Value> {
    let verified = BTreeMap::from([(from.to_owned(), value)]);
    derive(catalog, &[], &verified, &BTreeSet::new())
        .into_iter()
        .map(|d| {
            assert_eq!(d.from, from);
            (d.id, d.value)
        })
        .collect()
}

fn expect<S: AsRef<str>>(pairs: &[(S, bool)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(id, v)| (id.as_ref().to_owned(), Value::Boolean(*v)))
        .collect()
}

#[test]
fn intake_set_is_the_approved_first_22() {
    let catalog = common::bundled();
    let intake = catalog.intake();
    let first: Vec<(u32, &str)> = intake
        .iter()
        .take(INTAKE.len())
        .map(|d| (d.intake_rank.unwrap(), d.id.as_str()))
        .collect();
    let want: Vec<(u32, &str)> = INTAKE
        .iter()
        .enumerate()
        .map(|(i, id)| (i as u32 + 1, *id))
        .collect();
    assert_eq!(first, want);
    // Ranks above 22 belong to sections G to K; none of A to F may hold one.
    for d in intake.iter().skip(INTAKE.len()) {
        assert!(!in_a_to_f(&d.id), "{} must not have an intake rank", d.id);
    }
}

#[test]
fn intake_questions_are_plain_asks() {
    let catalog = common::bundled();
    for id in INTAKE {
        let d = catalog.get(id).unwrap();
        assert!(!d.optional, "{id} is optional");
        assert!(d.default_policy.is_none(), "{id} has a default policy");
    }
}

#[test]
fn intake_types_match_the_intake_doc() {
    use gantry_core::questions::AnswerType::{Date, Enum, Number};
    let catalog = common::bundled();
    let ty = |id: &str| catalog.get(id).unwrap().answer_type;
    assert_eq!(ty("profile.referral.default_source"), Enum);
    assert_eq!(ty("availability.start.earliest_date"), Date);
    assert_eq!(ty("availability.work_mode"), Enum);
    assert_eq!(ty("availability.pay.desired_annual"), Number);
    assert_eq!(
        catalog
            .get("availability.pay.desired_annual")
            .unwrap()
            .sensitivity,
        Sensitivity::Numeric
    );
    let levels = &catalog
        .get("profile.education.highest_level")
        .unwrap()
        .options;
    assert_eq!(
        levels.as_slice(),
        [
            "none",
            "high_school",
            "some_college",
            "certificate",
            "associate",
            "bachelors",
            "masters",
            "professional",
            "doctorate"
        ]
    );
}

#[test]
fn us_citizen_derives_five_answers() {
    let catalog = common::bundled();
    let got = suggest(
        &catalog,
        &format!("{AUTH}.us_citizen"),
        Value::Boolean(true),
    );
    assert_eq!(
        got,
        expect(&[
            (format!("{AUTH}.authorized_us"), true),
            (format!("{AUTH}.require_sponsorship_now"), false),
            (format!("{AUTH}.require_sponsorship_future"), false),
            (format!("{AUTH}.us_person"), true),
            (format!("{AUTH}.us_permanent_resident"), false),
        ])
    );
}

#[test]
fn permanent_resident_derives_four_answers() {
    let catalog = common::bundled();
    let got = suggest(
        &catalog,
        &format!("{AUTH}.us_permanent_resident"),
        Value::Boolean(true),
    );
    assert_eq!(
        got,
        expect(&[
            (format!("{AUTH}.authorized_us"), true),
            (format!("{AUTH}.require_sponsorship_now"), false),
            (format!("{AUTH}.require_sponsorship_future"), false),
            (format!("{AUTH}.us_person"), true),
        ])
    );
}

#[test]
fn not_a_citizen_or_resident_derives_nothing() {
    let catalog = common::bundled();
    for from in ["us_citizen", "us_permanent_resident"] {
        let got = suggest(&catalog, &format!("{AUTH}.{from}"), Value::Boolean(false));
        assert!(got.is_empty(), "{from} = false derived {got:?}");
    }
}

#[test]
fn age_rules() {
    let catalog = common::bundled();
    assert_eq!(
        suggest(&catalog, "legal.age.over_18", Value::Boolean(true)),
        expect(&[("legal.age.over_16", true)])
    );
    assert_eq!(
        suggest(&catalog, "legal.age.over_18", Value::Boolean(false)),
        expect(&[("legal.age.over_21", false)])
    );
}

#[test]
fn education_level_decides_has_bachelors() {
    let catalog = common::bundled();
    let levels = [
        ("none", false),
        ("high_school", false),
        ("some_college", false),
        ("certificate", false),
        ("associate", false),
        ("bachelors", true),
        ("masters", true),
        ("professional", true),
        ("doctorate", true),
    ];
    for (level, has) in levels {
        let got = suggest(
            &catalog,
            "profile.education.highest_level",
            Value::String(level.into()),
        );
        assert_eq!(
            got,
            expect(&[("profile.education.has_bachelors", has)]),
            "{level}"
        );
    }
}

#[test]
fn remote_only_means_not_willing_onsite() {
    let catalog = common::bundled();
    assert_eq!(
        suggest(
            &catalog,
            "availability.work_mode",
            Value::String("remote_only".into())
        ),
        expect(&[("availability.willing_onsite", false)])
    );
    for mode in ["hybrid", "onsite", "flexible"] {
        let got = suggest(
            &catalog,
            "availability.work_mode",
            Value::String(mode.into()),
        );
        assert!(got.is_empty(), "{mode} derived {got:?}");
    }
}

#[test]
fn only_the_approved_derive_rules_exist() {
    let catalog = common::bundled();
    let mut got: BTreeSet<(String, String)> = BTreeSet::new();
    for d in catalog.iter().filter(|d| in_a_to_f(&d.id)) {
        for rule in &d.derive {
            got.insert((rule.from.clone(), d.id.clone()));
        }
    }
    let mut want: BTreeSet<(String, String)> = BTreeSet::new();
    let mut add = |from: &str, to: &str| {
        want.insert((from.to_owned(), to.to_owned()));
    };
    for to in [
        "authorized_us",
        "require_sponsorship_now",
        "require_sponsorship_future",
        "us_person",
    ] {
        add(&format!("{AUTH}.us_citizen"), &format!("{AUTH}.{to}"));
        add(
            &format!("{AUTH}.us_permanent_resident"),
            &format!("{AUTH}.{to}"),
        );
    }
    add(
        &format!("{AUTH}.us_citizen"),
        &format!("{AUTH}.us_permanent_resident"),
    );
    add("legal.age.over_18", "legal.age.over_16");
    add("legal.age.over_18", "legal.age.over_21");
    add(
        "profile.education.highest_level",
        "profile.education.has_bachelors",
    );
    add("availability.work_mode", "availability.willing_onsite");
    assert_eq!(got, want);
}

#[test]
fn every_eeo_question_defaults_to_decline() {
    let catalog = common::bundled();
    let eeo: Vec<_> = catalog.iter().filter(|d| d.section == Section::E).collect();
    assert!(eeo.len() >= 10, "only {} EEO questions", eeo.len());
    for d in &eeo {
        assert!(d.id.starts_with("eeo."), "{}", d.id);
        assert_eq!(d.default_policy, Some(PolicyMode::Decline), "{}", d.id);
        assert_eq!(d.sensitivity, Sensitivity::Eeo, "{}", d.id);
        assert!(d.intake_rank.is_none(), "{}", d.id);
    }
    let in_file = catalog.iter().filter(|d| d.id.starts_with("eeo.")).count();
    assert_eq!(in_file, eeo.len());
}

#[test]
fn every_clearance_question_is_clearance() {
    let catalog = common::bundled();
    let d: Vec<_> = catalog.iter().filter(|d| d.section == Section::D).collect();
    assert!(d.len() >= 8, "only {} section D questions", d.len());
    for q in d {
        assert_eq!(q.sensitivity, Sensitivity::Clearance, "{}", q.id);
        assert!(q.id.starts_with("legal.clearance."), "{}", q.id);
    }
}

#[test]
fn always_ask_defaults() {
    let catalog = common::bundled();
    let got: BTreeSet<&str> = catalog
        .iter()
        .filter(|d| in_a_to_f(&d.id) && d.default_policy == Some(PolicyMode::AlwaysAsk))
        .map(|d| d.id.as_str())
        .collect();
    let want: BTreeSet<&str> = [
        // criminal history
        "legal.criminal.convicted",
        "legal.criminal.pending_charges",
        "legal.criminal.details",
        // military service
        "legal.military.served",
        "legal.military.service",
        // driving record
        "availability.driving.clean_record",
        // essential functions, physical requirements, interview accommodations
        "availability.functions.essential",
        "availability.physical.requirements",
        "availability.interview.accommodations",
        // references and pay history
        "availability.references.list",
        "availability.pay.current_annual",
    ]
    .into();
    assert_eq!(got, want);
}

#[test]
fn only_eeo_declines_by_default_and_nothing_draws_on() {
    let catalog = common::bundled();
    for d in catalog.iter().filter(|d| in_a_to_f(&d.id)) {
        if !d.id.starts_with("eeo.") {
            assert_ne!(d.default_policy, Some(PolicyMode::Decline), "{}", d.id);
        }
        assert!(d.draws_on.is_empty(), "{} has draws_on", d.id);
    }
}

#[test]
fn grids_are_only_in_availability() {
    let catalog = common::bundled();
    let grids: Vec<&str> = catalog
        .iter()
        .filter(|d| d.answer_type == gantry_core::questions::AnswerType::Grid)
        .map(|d| d.id.as_str())
        .collect();
    assert!(grids.contains(&"availability.schedule.weekly"));
    assert!(grids.iter().all(|id| id.starts_with("availability.")));
}
