//! The M1 bank structs and the question catalog describe the same fields
//! (design §9): every M1 path has a question of the matching type, enum
//! options equal the serde names, and a value written through the store
//! reads back through both the typed snapshot and the question states.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use gantry_core::bank::{ClearanceStatus, EducationLevel, Item};
use gantry_core::facts::ClearanceLevel;
use gantry_core::questions::{AnswerState, AnswerType};
use gantry_store::{bundled_catalog, load_bank_docs, load_snapshot, update_table};
use serde::Serialize;

fn names<T: Serialize>(variants: &[T]) -> BTreeSet<String> {
    variants
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn every_m1_field_has_a_question_of_the_same_type() {
    let catalog = bundled_catalog().unwrap();
    let expect = [
        ("profile.education.highest_level", AnswerType::Enum),
        (
            "legal.authorization.require_sponsorship_now",
            AnswerType::Bool,
        ),
        (
            "legal.authorization.require_sponsorship_future",
            AnswerType::Bool,
        ),
        ("legal.authorization.us_citizen", AnswerType::Bool),
        ("legal.clearance.level", AnswerType::Enum),
        ("legal.clearance.status", AnswerType::Enum),
        ("legal.clearance.willing_to_obtain", AnswerType::Bool),
        ("experience.licenses", AnswerType::List),
    ];
    for (id, answer_type) in expect {
        let def = catalog
            .get(id)
            .unwrap_or_else(|| panic!("no question {id}"));
        assert_eq!(def.answer_type, answer_type, "{id}");
    }
}

/// Fails to compile when an M1 enum gains a variant, so the lists in
/// `enum_options_equal_the_serde_names` cannot fall behind.
#[allow(dead_code)]
fn variant_lists_are_exhaustive(e: EducationLevel, l: ClearanceLevel, s: ClearanceStatus) {
    use EducationLevel as E;
    match e {
        E::None
        | E::HighSchool
        | E::SomeCollege
        | E::Certificate
        | E::Associate
        | E::Bachelors
        | E::Masters
        | E::Professional
        | E::Doctorate => {}
    }
    match l {
        ClearanceLevel::None
        | ClearanceLevel::PublicTrust
        | ClearanceLevel::Confidential
        | ClearanceLevel::Secret
        | ClearanceLevel::TopSecret
        | ClearanceLevel::TsSci => {}
    }
    match s {
        ClearanceStatus::Active | ClearanceStatus::Current | ClearanceStatus::Expired => {}
    }
}

#[test]
fn enum_options_equal_the_serde_names() {
    use EducationLevel as E;
    let catalog = bundled_catalog().unwrap();
    let education = names(&[
        E::None,
        E::HighSchool,
        E::SomeCollege,
        E::Certificate,
        E::Associate,
        E::Bachelors,
        E::Masters,
        E::Professional,
        E::Doctorate,
    ]);
    let level = names(&[
        ClearanceLevel::None,
        ClearanceLevel::PublicTrust,
        ClearanceLevel::Confidential,
        ClearanceLevel::Secret,
        ClearanceLevel::TopSecret,
        ClearanceLevel::TsSci,
    ]);
    let status = names(&[
        ClearanceStatus::Active,
        ClearanceStatus::Current,
        ClearanceStatus::Expired,
    ]);
    for (id, expected) in [
        ("profile.education.highest_level", education),
        ("legal.clearance.level", level),
        ("legal.clearance.status", status),
    ] {
        let options: BTreeSet<String> = catalog.get(id).unwrap().options.iter().cloned().collect();
        assert_eq!(options, expected, "{id}");
    }
}

#[test]
fn a_stored_answer_reads_back_typed_and_as_a_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legal.toml");
    let item: Item<bool> = toml::from_str(
        "id = \"legal.authorization.us_citizen\"\nvalue = true\nverified = true\n\
         created_at = \"2026-09-01T00:00:00Z\"\nupdated_at = \"2026-09-01T00:00:00Z\"\n",
    )
    .unwrap();
    update_table(&path, |doc| {
        let mut table = toml::Table::new();
        table.insert("us_citizen".into(), toml::Value::try_from(&item).unwrap());
        doc.insert("authorization".into(), toml::Value::Table(table));
    })
    .unwrap();

    let snapshot = load_snapshot(dir.path()).unwrap();
    assert_eq!(snapshot.banks.us_citizen(), Some(true));

    let catalog = bundled_catalog().unwrap();
    let states = load_bank_docs(dir.path()).unwrap().states(&catalog);
    assert_eq!(
        states["legal.authorization.us_citizen"],
        AnswerState::Answered
    );
    assert_eq!(
        states["legal.authorization.require_sponsorship_now"],
        AnswerState::Missing
    );
}
