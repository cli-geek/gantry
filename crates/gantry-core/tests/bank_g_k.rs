//! Sections G to K of the bundled bank: intake ids, projection map, story shape, technology pack.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use gantry_core::questions::{AnswerType, Catalog, ReviewStatus};

#[test]
fn intake_ranks_23_to_28() {
    let cat = common::bundled();
    let ids: Vec<&str> = cat
        .intake()
        .into_iter()
        .filter(|d| d.intake_rank.is_some_and(|r| r >= 23))
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "experience.total_years",
            "motivation.cover_letter_stance",
            "motivation.looking_now",
            "motivation.domains",
            "motivation.bio_short",
            "stories.proudest_work",
        ]
    );
    for (id, rank) in [
        ("experience.total_years", 23),
        ("motivation.cover_letter_stance", 24),
        ("motivation.looking_now", 25),
        ("motivation.domains", 26),
        ("motivation.bio_short", 27),
        ("stories.proudest_work", 28),
    ] {
        assert_eq!(cat.get(id).unwrap().intake_rank, Some(rank), "{id}");
    }
}

#[test]
fn projection_map_resolves() {
    let cat = common::bundled();
    let map: &[(&str, &[&str])] = &[
        (
            "motivation.why_company",
            &[
                "motivation.domains",
                "motivation.looking_now",
                "motivation.values",
            ],
        ),
        (
            "motivation.why_role",
            &[
                "motivation.looking_now",
                "motivation.goals.one_year",
                "motivation.goals.three_years",
                "motivation.goals.five_years",
                "stories.proudest_work",
            ],
        ),
        (
            "motivation.bio_150",
            &[
                "motivation.bio_short",
                "experience.total_years",
                "stories.proudest_work",
            ],
        ),
        (
            "stories.hardest_problem",
            &[
                "experience.total_years",
                "experience.jobs",
                "stories.proudest_work",
            ],
        ),
        (
            "approach.organize_work",
            &["stories.proudest_work", "stories.failure"],
        ),
    ];
    for (id, expected) in map {
        let got: Vec<&str> = cat.draws_on(id).iter().map(|d| d.id.as_str()).collect();
        assert!(!got.is_empty(), "{id} draws on nothing");
        for e in *expected {
            assert!(got.contains(e), "{id} should draw on {e}, got {got:?}");
        }
    }
}

#[test]
fn story_prompts_are_star_lists() {
    let cat = common::bundled();
    let stories: Vec<_> = cat
        .iter()
        .filter(|d| d.id.starts_with("stories."))
        .collect();
    assert_eq!(stories.len(), 20);
    for d in stories {
        assert_eq!(d.answer_type, AnswerType::List, "{}", d.id);
        assert_eq!(d.min_items, 1, "{}", d.id);
        let names: Vec<&str> = d.fields.iter().map(|f| f.name.as_str()).collect();
        for want in ["situation", "task", "action", "result"] {
            assert!(names.contains(&want), "{} lacks {want}", d.id);
        }
    }
}

#[test]
fn technology_pack_is_unreviewed() {
    let cat = common::bundled();
    let pack = cat.pack("technology").expect("technology pack");
    assert_eq!(pack.review_status, ReviewStatus::Unreviewed);
    assert!(pack.reviewed_by.is_empty());
    let tech: Vec<_> = cat
        .iter()
        .filter(|d| cat.pack_of(&d.id) == Some("technology"))
        .collect();
    assert!(tech.len() >= 20);
    assert!(
        tech.iter()
            .all(|d| d.id.starts_with("approach.technology."))
    );
}

#[test]
fn pack_template_parses() {
    let template = include_str!("../../../data/questions/packs/_template.toml");
    let cat = Catalog::parse(&[("packs/example.toml", template)]).unwrap();
    assert_eq!(cat.pack("example").unwrap().pack, "example");
    assert!(cat.get("approach.example.handoff").is_some());
}
