// Tests build profiles by mutating a default, mirroring how a user edits one section.
#![allow(clippy::field_reassign_with_default)]

use jiff::Timestamp;

use super::*;
use crate::bank::{ClearanceStatus, Item};
use crate::facts::{
    ClearanceLevel, ClearanceRequirement, Fact, FactSource, PostingFacts, YearsRequirement,
};
use crate::geo::{Distance, DistanceUnit};
use crate::search::{Synonym, Titles};
use crate::{Ats, PayPeriod, PostingKey};

const NOW: i64 = 1_790_000_000;
const DAY: i64 = 86_400;

fn place(name: &str, admin1: &str, lat: f64, lon: f64) -> Place {
    Place {
        name: name.into(),
        admin1: Some(admin1.into()),
        country: "US".into(),
        lat,
        lon,
    }
}

fn seattle() -> Place {
    place("Seattle", "WA", 47.6062, -122.3321)
}

fn bellevue() -> Place {
    place("Bellevue", "WA", 47.6101, -122.2015)
}

fn portland() -> Place {
    place("Portland", "OR", 45.5152, -122.6784)
}

fn posting(title: &str) -> Posting {
    Posting {
        key: PostingKey {
            ats: Ats::Greenhouse,
            board_token: "acme".into(),
            job_id: "1".into(),
        },
        company_name: "Acme".into(),
        title: title.into(),
        url: "https://example.test/1".into(),
        locations_raw: vec![],
        locations: vec![],
        work_mode: None,
        job_type: None,
        published_at: Some(NOW - 2 * DAY),
        description: String::new(),
        facts: PostingFacts::default(),
    }
}

fn at(p: Place, modes: &[WorkMode]) -> Location {
    Location {
        raw: p.name.clone(),
        modes: modes.to_vec(),
        place: Some(p),
        area: None,
        unresolved: None,
    }
}

fn remote_in(area: Option<Area>) -> Location {
    Location {
        raw: "Remote".into(),
        modes: vec![WorkMode::Remote],
        place: None,
        area,
        unresolved: None,
    }
}

fn item<T>(value: T) -> Option<Item<T>> {
    Some(Item {
        id: "i".into(),
        value,
        verified: true,
        created_at: Timestamp::UNIX_EPOCH,
        updated_at: Timestamp::UNIX_EPOCH,
    })
}

fn pattern_fact<T>(value: T) -> Option<Fact<T>> {
    Some(Fact {
        value,
        source: FactSource::Pattern,
        evidence: "evidence".into(),
    })
}

struct Fixture {
    search: SearchProfile,
    banks: Banks,
    home: Option<Place>,
    relocate: Vec<Place>,
}

impl Fixture {
    fn new(search: SearchProfile) -> Self {
        Self {
            search,
            banks: Banks::default(),
            home: Some(seattle()),
            relocate: vec![],
        }
    }

    fn run(&self, posting: &Posting) -> FilterOutcome {
        let ctx = FilterContext {
            search: &self.search,
            banks: &self.banks,
            home: self.home.as_ref(),
            relocate_places: &self.relocate,
            now: NOW,
        };
        evaluate(
            &Subject {
                posting,
                first_seen: NOW,
                staffing_agency: None,
            },
            &ctx,
        )
    }
}

fn only(outcome: &FilterOutcome, filter: FilterName) -> &Check {
    let found: Vec<_> = outcome
        .checks
        .iter()
        .filter(|c| c.filter == filter)
        .collect();
    assert_eq!(found.len(), 1, "{outcome:#?}");
    found[0]
}

#[test]
fn empty_profile_filters_nothing() {
    let f = Fixture::new(SearchProfile::default());
    let out = f.run(&posting("Anything At All"));
    assert!(out.passed);
    assert!(out.checks.is_empty(), "{out:#?}");
}

#[test]
fn title_targets_synonyms_and_excludes() {
    let mut search = SearchProfile::default();
    search.titles = Titles {
        mode: Mode::Hard,
        targets: vec!["Dental Hygienist".into()],
        synonyms: vec![
            Synonym {
                text: "Hygienist".into(),
                verified: true,
            },
            Synonym {
                text: "Dental Assistant".into(),
                verified: false,
            },
        ],
        exclude: vec!["Manager".into()],
    };
    let f = Fixture::new(search);
    assert!(f.run(&posting("Registered Dental Hygienist")).passed);
    assert!(f.run(&posting("Hygienist (Part-Time)")).passed);
    // Unverified synonym is not used.
    assert!(!f.run(&posting("Dental Assistant")).passed);
    let out = f.run(&posting("Dental Hygienist Manager"));
    assert!(!out.passed);
    assert!(only(&out, FilterName::Title).detail.contains("Manager"));
}

#[test]
fn exclude_terms_apply_even_when_mode_is_any() {
    let mut search = SearchProfile::default();
    search.titles.exclude = vec!["Senior".into()];
    let f = Fixture::new(search);
    assert!(!f.run(&posting("Senior Engineer")).passed);
    assert!(f.run(&posting("Engineer")).passed);
}

#[test]
fn location_distance_modes_and_relocation() {
    let mut search = SearchProfile::default();
    search.location.mode = Mode::Hard;
    search.location.max_distance = Some(Distance {
        value: 25.0,
        unit: DistanceUnit::Mi,
    });
    search.location.work_modes = vec![WorkMode::Onsite, WorkMode::Hybrid];
    let mut f = Fixture::new(search);

    let mut near = posting("Job");
    near.locations = vec![at(bellevue(), &[])];
    let out = f.run(&near);
    assert!(out.passed, "{out:#?}");
    assert!(
        only(&out, FilterName::Location)
            .detail
            .contains("mi from home")
    );

    let mut far = posting("Job");
    far.locations = vec![at(portland(), &[WorkMode::Hybrid])];
    assert!(!f.run(&far).passed);

    // Any one acceptable location is enough.
    far.locations.push(at(bellevue(), &[]));
    assert!(f.run(&far).passed);

    let mut remote = posting("Job");
    remote.locations = vec![remote_in(None)];
    assert!(!f.run(&remote).passed, "remote is not an accepted mode");

    f.search.location.relocate.willing = RelocateWillingness::Listed;
    f.relocate = vec![place("Portland", "OR", 45.52, -122.68)];
    let mut pdx = posting("Job");
    pdx.locations = vec![at(portland(), &[])];
    assert!(f.run(&pdx).passed);
}

#[test]
fn structured_work_mode_applies_to_locations_without_one() {
    let mut search = SearchProfile::default();
    search.location.mode = Mode::Hard;
    search.location.work_modes = vec![WorkMode::Remote];
    let f = Fixture::new(search);
    let mut p = posting("Job");
    p.locations = vec![at(portland(), &[])];
    assert!(!f.run(&p).passed, "no mode stated → assumed on-site");
    p.work_mode = Some(WorkMode::Remote);
    assert!(f.run(&p).passed);
}

#[test]
fn unresolved_place_is_unknown_not_excluded() {
    let mut search = SearchProfile::default();
    search.location.mode = Mode::Hard;
    search.location.max_distance = Some(Distance {
        value: 10.0,
        unit: DistanceUnit::Km,
    });
    let f = Fixture::new(search);
    let mut p = posting("Job");
    p.locations = vec![Location {
        raw: "Springfield Campus".into(),
        modes: vec![],
        place: None,
        area: None,
        unresolved: Some("Springfield Campus".into()),
    }];
    let out = f.run(&p);
    assert!(out.passed);
    assert_eq!(only(&out, FilterName::Location).verdict, Verdict::Unknown);
}

#[test]
fn remote_eligibility_by_country_and_region() {
    let mut search = SearchProfile::default();
    search.location.mode = Mode::Hard;
    search.location.remote_eligible = vec!["US-NY".into()];
    let f = Fixture::new(search);
    let country = |c: &str| Some(Area::Country { country: c.into() });
    let admin = |a: &str| {
        Some(Area::Admin1 {
            country: "US".into(),
            admin1: a.into(),
        })
    };
    let run = |area| {
        let mut p = posting("Job");
        p.locations = vec![remote_in(area)];
        f.run(&p)
    };
    assert!(run(country("US")).passed);
    assert!(!run(country("CA")).passed);
    assert!(run(admin("NY")).passed);
    assert!(!run(admin("CA")).passed);
    assert!(run(Some(Area::Anywhere)).passed);
    let out = run(None);
    assert!(out.passed);
    assert_eq!(only(&out, FilterName::Location).verdict, Verdict::Unknown);
}

#[test]
fn pay_floor_normalizes_periods_and_keeps_unlisted() {
    let mut search = SearchProfile::default();
    search.pay.mode = Mode::Hard;
    search.pay.minimum = Some(40.0);
    search.pay.period = Some(PayPeriod::Hour);
    search.pay.currency = Some("USD".into());
    let mut f = Fixture::new(search);
    let mut p = posting("Job");
    let pay = |min: f64, max: f64, period: PayPeriod, currency: &str| {
        pattern_fact(Pay {
            min: Some(min),
            max: Some(max),
            currency: currency.into(),
            period,
        })
    };

    p.facts.pay = pay(90_000.0, 95_000.0, PayPeriod::Year, "USD");
    assert!(f.run(&p).passed, "95k/yr ≥ 40/h × 2080 = 83.2k");
    p.facts.pay = pay(30.0, 38.0, PayPeriod::Hour, "USD");
    assert!(!f.run(&p).passed);
    p.facts.pay = pay(30.0, 38.0, PayPeriod::Hour, "CAD");
    assert_eq!(only(&f.run(&p), FilterName::Pay).verdict, Verdict::Unknown);

    p.facts.pay = None;
    assert!(f.run(&p).passed);
    f.search.pay.keep_unlisted = false;
    assert!(!f.run(&p).passed);
}

#[test]
fn experience_preferred_years_are_soft() {
    let mut search = SearchProfile::default();
    search.experience.mode = Mode::Hard;
    search.experience.max_required_years = Some(2);
    let mut f = Fixture::new(search);
    let mut p = posting("Job");
    p.facts.min_years = pattern_fact(YearsRequirement {
        years: 5,
        preferred: true,
    });
    assert!(f.run(&p).passed);
    f.search.experience.preferred_is_soft = false;
    assert!(!f.run(&p).passed);
    p.facts.min_years = None;
    assert!(f.run(&p).passed);
}

#[test]
fn sponsorship_uses_only_verified_answers() {
    let mut search = SearchProfile::default();
    search.requirements.sponsorship = Mode::Hard;
    let mut f = Fixture::new(search);
    let mut p = posting("Job");
    p.facts.sponsorship = pattern_fact(Sponsorship::NotOffered);

    let out = f.run(&p);
    assert!(out.passed);
    assert_eq!(
        only(&out, FilterName::Sponsorship).verdict,
        Verdict::Unknown
    );

    f.banks.legal.authorization.require_sponsorship_now = item(true);
    assert!(!f.run(&p).passed);

    p.facts.sponsorship = pattern_fact(Sponsorship::CitizenshipRequired);
    f.banks.legal.authorization.us_citizen = item(false);
    assert!(!f.run(&p).passed);
}

#[test]
fn clearance_held_or_obtainable() {
    let mut search = SearchProfile::default();
    search.requirements.clearance = Mode::Hard;
    let mut f = Fixture::new(search);
    f.banks.legal.clearance.level = item(ClearanceLevel::None);
    let mut p = posting("Job");
    p.facts.clearance = pattern_fact(ClearanceRequirement {
        level: ClearanceLevel::Secret,
        active_required: false,
    });
    assert!(!f.run(&p).passed);
    f.banks.legal.clearance.willing_to_obtain = item(true);
    assert!(f.run(&p).passed);

    p.facts.clearance = pattern_fact(ClearanceRequirement {
        level: ClearanceLevel::TsSci,
        active_required: true,
    });
    assert!(!f.run(&p).passed);
    f.banks.legal.clearance.level = item(ClearanceLevel::TsSci);
    f.banks.legal.clearance.status = item(ClearanceStatus::Active);
    assert!(f.run(&p).passed);
}

#[test]
fn freshness_falls_back_to_first_seen() {
    let mut search = SearchProfile::default();
    search.freshness.max_age_days = Some(7);
    let f = Fixture::new(search);
    let mut p = posting("Job");
    p.published_at = Some(NOW - 30 * DAY);
    assert!(!f.run(&p).passed);
    p.published_at = None;
    assert!(f.run(&p).passed);
}

#[test]
fn keywords_and_company_lists() {
    let mut search = SearchProfile::default();
    search.keywords.mode = Mode::Hard;
    search.keywords.exclude = vec!["commission".into()];
    search.companies.block = vec!["ACME".into()];
    let mut f = Fixture::new(search);
    let mut p = posting("Job");
    assert!(!f.run(&p).passed, "blocked company");
    p.company_name = "Other Co".into();
    assert!(f.run(&p).passed);
    p.description = "Base pay plus commission.".into();
    assert!(!f.run(&p).passed);

    f.search.keywords = Default::default();
    f.search.companies.allow_mode = Mode::Hard;
    f.search.companies.allow = vec!["Northwind".into()];
    assert!(!f.run(&p).passed);
}

#[test]
fn preference_mode_records_but_never_excludes() {
    let mut search = SearchProfile::default();
    search.titles.mode = Mode::Prefer;
    search.titles.targets = vec!["Nurse".into()];
    let f = Fixture::new(search);
    let out = f.run(&posting("Barista"));
    assert!(out.passed);
    assert_eq!(only(&out, FilterName::Title).verdict, Verdict::Fail);
}
