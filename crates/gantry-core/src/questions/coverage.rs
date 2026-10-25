//! Coverage meter (design §6): per-question answer state from what is stored
//! and the user's policy, rolled up into tallies by section and pack.
//!
//! Pure functions over data the caller loaded; no values are read, only the
//! shape of what is stored.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Catalog, PolicyMode, QuestionDef, Section};

/// How many entries `top_missing` lists.
const TOP_MISSING: usize = 10;

/// Where one question stands.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AnswerState {
    Answered,
    /// Stored but not yet verified; counts as not covered.
    Suggested,
    Declined,
    AlwaysAsk,
    NotApplicable,
    Missing,
    /// Nothing stored and no policy, but at least one `draws_on` source is
    /// answered, so M3 can compose it; counts as covered.
    Composable,
}

/// What is stored for a question, without the value itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Scalar {
        verified: bool,
    },
    /// A list: how many entries, and how many of them are verified.
    List {
        items: usize,
        verified: usize,
    },
}

/// State of one question. Precedence: an explicit `policy` (even over a
/// stored value), then the stored value, then the definition's
/// `default_policy`, then missing.
pub fn state_of(
    def: &QuestionDef,
    stored: Option<Stored>,
    policy: Option<PolicyMode>,
) -> AnswerState {
    if let Some(mode) = policy {
        return from_policy(mode);
    }
    match stored {
        Some(Stored::Scalar { verified: true }) => return AnswerState::Answered,
        Some(Stored::Scalar { verified: false }) => return AnswerState::Suggested,
        Some(Stored::List { items, verified }) => {
            let need = (def.min_items as usize).max(1);
            if verified >= need {
                return AnswerState::Answered;
            }
            if items > 0 {
                return AnswerState::Suggested;
            }
        }
        None => {}
    }
    def.default_policy.map_or(AnswerState::Missing, from_policy)
}

fn from_policy(mode: PolicyMode) -> AnswerState {
    match mode {
        PolicyMode::Decline => AnswerState::Declined,
        PolicyMode::AlwaysAsk => AnswerState::AlwaysAsk,
        PolicyMode::NotApplicable => AnswerState::NotApplicable,
    }
}

/// Counts for a group of applicable questions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
pub struct Tally {
    pub answered: u32,
    pub suggested: u32,
    pub declined: u32,
    pub always_ask: u32,
    pub not_applicable: u32,
    pub missing: u32,
    pub composable: u32,
    /// Answered + declined + not applicable + composable.
    pub covered: u32,
    /// All questions except always-ask ones.
    pub denominator: u32,
    /// `None` when the denominator is 0.
    pub ratio: Option<f64>,
    pub weighted_covered: u64,
    pub weighted_denominator: u64,
    /// `None` when the weighted denominator is 0.
    pub weighted_ratio: Option<f64>,
}

impl Tally {
    fn add(&mut self, state: AnswerState, weight: u32) {
        let w = u64::from(weight);
        let covered = matches!(
            state,
            AnswerState::Answered
                | AnswerState::Declined
                | AnswerState::NotApplicable
                | AnswerState::Composable
        );
        match state {
            AnswerState::Answered => self.answered += 1,
            AnswerState::Suggested => self.suggested += 1,
            AnswerState::Declined => self.declined += 1,
            AnswerState::AlwaysAsk => self.always_ask += 1,
            AnswerState::NotApplicable => self.not_applicable += 1,
            AnswerState::Missing => self.missing += 1,
            AnswerState::Composable => self.composable += 1,
        }
        if covered {
            self.covered += 1;
            self.weighted_covered += w;
        }
        if state != AnswerState::AlwaysAsk {
            self.denominator += 1;
            self.weighted_denominator += w;
        }
    }

    /// Fill in the ratios once every question has been added.
    fn finish(&mut self) {
        self.ratio =
            (self.denominator > 0).then(|| f64::from(self.covered) / f64::from(self.denominator));
        self.weighted_ratio = (self.weighted_denominator > 0)
            .then(|| self.weighted_covered as f64 / self.weighted_denominator as f64);
    }
}

/// An applicable question that still needs the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MissingEntry {
    pub id: String,
    pub label: String,
    pub section: Section,
    pub weight: u32,
    /// `missing` or `suggested`.
    pub state: AnswerState,
}

/// The coverage meter's output.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CoverageReport {
    pub overall: Tally,
    /// Only sections that have applicable questions.
    pub sections: BTreeMap<Section, Tally>,
    /// `core`, plus each pack that has applicable questions.
    pub packs: BTreeMap<String, Tally>,
    /// Up to 10 missing or suggested questions, heaviest first.
    pub top_missing: Vec<MissingEntry>,
}

/// Roll `states` up over the questions that apply to `occupations`. Ids
/// absent from `states` are [`AnswerState::Missing`], or
/// [`AnswerState::Composable`] when [`composable`] says so.
pub fn coverage(
    catalog: &Catalog,
    occupations: &[String],
    states: &BTreeMap<String, AnswerState>,
) -> CoverageReport {
    let mut overall = Tally::default();
    let mut sections: BTreeMap<Section, Tally> = BTreeMap::new();
    let mut packs: BTreeMap<String, Tally> = BTreeMap::new();
    let mut open = Vec::new();

    for def in catalog.iter() {
        if !def.is_addressable() || def.optional {
            continue;
        }
        let pack = catalog.pack_of(&def.id);
        if pack.is_some_and(|p| !occupations.iter().any(|o| o == p)) {
            continue;
        }
        let mut state = states.get(&def.id).copied().unwrap_or(AnswerState::Missing);
        if state == AnswerState::Missing && composable(catalog, def, states) {
            state = AnswerState::Composable;
        }
        overall.add(state, def.weight);
        sections
            .entry(def.section)
            .or_default()
            .add(state, def.weight);
        packs
            .entry(pack.unwrap_or("core").to_owned())
            .or_default()
            .add(state, def.weight);
        if matches!(state, AnswerState::Missing | AnswerState::Suggested) {
            open.push(MissingEntry {
                id: def.id.clone(),
                label: def.label.clone(),
                section: def.section,
                weight: def.weight,
                state,
            });
        }
    }

    overall.finish();
    sections.values_mut().for_each(Tally::finish);
    packs.values_mut().for_each(Tally::finish);
    open.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.id.cmp(&b.id)));
    open.truncate(TOP_MISSING);

    CoverageReport {
        overall,
        sections,
        packs,
        top_missing: open,
    }
}

/// Whether `def` can be composed: it has `draws_on` and at least one of
/// the questions those entries match is answered (verified). The caller
/// applies this only to a missing question.
pub fn composable(
    catalog: &Catalog,
    def: &QuestionDef,
    states: &BTreeMap<String, AnswerState>,
) -> bool {
    !def.draws_on.is_empty()
        && catalog
            .draws_on(&def.id)
            .iter()
            .any(|src| states.get(&src.id) == Some(&AnswerState::Answered))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `[[question]]` table; `extra` is appended as more keys.
    fn q(id: &str, section: &str, sens: &str, extra: &str) -> String {
        format!(
            "[[question]]\nid = '{id}'\nsection = '{section}'\nlabel = 'L {id}'\n\
             sensitivity = '{sens}'\nanswer_type = 'text'\n{extra}\n"
        )
    }

    fn pack_file(name: &str, body: &str) -> String {
        format!("pack = '{name}'\nreview_status = 'unreviewed'\nreviewed_by = []\n{body}")
    }

    fn one(extra: &str) -> QuestionDef {
        let cat = Catalog::parse(&[("legal.toml", &q("legal.a", "C", "legal", extra))]).unwrap();
        cat.get("legal.a").unwrap().clone()
    }

    fn states(pairs: &[(&str, AnswerState)]) -> BTreeMap<String, AnswerState> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    const VERIFIED: Option<Stored> = Some(Stored::Scalar { verified: true });

    #[test]
    fn policy_beats_stored_value() {
        let d = one("");
        for (mode, want) in [
            (PolicyMode::Decline, AnswerState::Declined),
            (PolicyMode::AlwaysAsk, AnswerState::AlwaysAsk),
            (PolicyMode::NotApplicable, AnswerState::NotApplicable),
        ] {
            assert_eq!(state_of(&d, VERIFIED, Some(mode)), want);
            assert_eq!(state_of(&d, None, Some(mode)), want);
        }
    }

    #[test]
    fn policy_beats_default_policy() {
        let d = one("default_policy = 'decline'");
        assert_eq!(
            state_of(&d, None, Some(PolicyMode::AlwaysAsk)),
            AnswerState::AlwaysAsk
        );
    }

    #[test]
    fn scalar_states() {
        let d = one("default_policy = 'decline'");
        assert_eq!(state_of(&d, VERIFIED, None), AnswerState::Answered);
        assert_eq!(
            state_of(&d, Some(Stored::Scalar { verified: false }), None),
            AnswerState::Suggested
        );
    }

    #[test]
    fn default_policy_and_missing() {
        assert_eq!(
            state_of(&one("default_policy = 'decline'"), None, None),
            AnswerState::Declined
        );
        assert_eq!(
            state_of(&one("default_policy = 'always_ask'"), None, None),
            AnswerState::AlwaysAsk
        );
        assert_eq!(state_of(&one(""), None, None), AnswerState::Missing);
    }

    #[test]
    fn list_states() {
        let list = |items, verified| Some(Stored::List { items, verified });
        let d = one("");
        assert_eq!(state_of(&d, list(1, 1), None), AnswerState::Answered);
        assert_eq!(state_of(&d, list(3, 1), None), AnswerState::Answered);
        assert_eq!(state_of(&d, list(2, 0), None), AnswerState::Suggested);
        assert_eq!(state_of(&d, list(0, 0), None), AnswerState::Missing);

        // min_items = 2: one verified entry is not enough.
        let cat = Catalog::parse(&[(
            "profile.toml",
            &q("profile.refs", "B", "contact", "min_items = 2").replace("'text'", "'list'"),
        )])
        .unwrap();
        let d = cat.get("profile.refs").unwrap();
        assert_eq!(state_of(d, list(2, 2), None), AnswerState::Answered);
        assert_eq!(state_of(d, list(2, 1), None), AnswerState::Suggested);
        assert_eq!(state_of(d, list(1, 1), None), AnswerState::Suggested);
        assert_eq!(state_of(d, list(0, 0), None), AnswerState::Missing);
    }

    #[test]
    fn empty_list_falls_through_to_default_policy() {
        let d = one("default_policy = 'always_ask'");
        assert_eq!(
            state_of(
                &d,
                Some(Stored::List {
                    items: 0,
                    verified: 0
                }),
                None
            ),
            AnswerState::AlwaysAsk
        );
    }

    #[test]
    fn answer_state_serializes_snake_case() {
        let v = toml::Value::try_from(AnswerState::AlwaysAsk).unwrap();
        assert_eq!(v.as_str(), Some("always_ask"));
    }

    #[test]
    fn empty_catalog_has_no_ratios() {
        let cat = Catalog::parse(&[]).unwrap();
        let r = coverage(&cat, &[], &BTreeMap::new());
        assert_eq!(r.overall, Tally::default());
        assert_eq!(r.overall.ratio, None);
        assert_eq!(r.overall.weighted_ratio, None);
        assert!(r.sections.is_empty() && r.packs.is_empty() && r.top_missing.is_empty());
    }

    #[test]
    fn all_always_ask_has_no_ratio() {
        let body = q("legal.a", "C", "legal", "");
        let cat = Catalog::parse(&[("legal.toml", &body)]).unwrap();
        let r = coverage(&cat, &[], &states(&[("legal.a", AnswerState::AlwaysAsk)]));
        assert_eq!(r.overall.always_ask, 1);
        assert_eq!(r.overall.denominator, 0);
        assert_eq!(r.overall.ratio, None);
        assert_eq!(r.overall.weighted_ratio, None);
    }

    /// Hand-computed example. Weights: a=1, b=3, c=2, d=4, e=5.
    /// a answered, b declined, c missing, d suggested, e always_ask.
    /// covered = 2 of denominator 4 -> 0.5; weighted 1+3=4 of 1+3+2+4=10 -> 0.4.
    #[test]
    fn weighted_and_unweighted_ratios() {
        let body = [
            q("legal.a", "C", "legal", ""),
            q("legal.b", "C", "legal", "weight = 3"),
            q("legal.c", "C", "legal", "weight = 2"),
            q("legal.d", "C", "legal", "weight = 4"),
            q("legal.e", "C", "legal", "weight = 5"),
        ]
        .concat();
        let cat = Catalog::parse(&[("legal.toml", &body)]).unwrap();
        let st = states(&[
            ("legal.a", AnswerState::Answered),
            ("legal.b", AnswerState::Declined),
            ("legal.d", AnswerState::Suggested),
            ("legal.e", AnswerState::AlwaysAsk),
        ]);
        let t = coverage(&cat, &[], &st).overall;
        assert_eq!(
            (t.answered, t.declined, t.suggested, t.missing, t.always_ask),
            (1, 1, 1, 1, 1)
        );
        assert_eq!((t.covered, t.denominator), (2, 4));
        assert_eq!(t.ratio, Some(0.5));
        assert_eq!((t.weighted_covered, t.weighted_denominator), (4, 10));
        assert_eq!(t.weighted_ratio, Some(0.4));
    }

    #[test]
    fn not_applicable_is_covered() {
        let cat = Catalog::parse(&[("legal.toml", &q("legal.a", "C", "legal", ""))]).unwrap();
        let t = coverage(
            &cat,
            &[],
            &states(&[("legal.a", AnswerState::NotApplicable)]),
        )
        .overall;
        assert_eq!((t.covered, t.denominator, t.ratio), (1, 1, Some(1.0)));
    }

    fn mixed() -> Catalog {
        let trades = pack_file("trades", &q("approach.trades.a", "I", "free_text", ""));
        Catalog::parse(&[
            (
                "legal.toml",
                &[
                    q("legal.a", "C", "legal", ""),
                    q("legal.opt", "C", "legal", "optional = true"),
                    q("legal.attest", "C", "attestation", ""),
                ]
                .concat(),
            ),
            (
                "profile.toml",
                &q("profile.name", "A", "contact", "weight = 2"),
            ),
            ("packs/trades.toml", &trades),
        ])
        .unwrap()
    }

    #[test]
    fn pack_questions_follow_occupations() {
        let cat = mixed();
        let none = coverage(&cat, &[], &BTreeMap::new());
        assert_eq!(none.overall.denominator, 2);
        assert_eq!(none.packs.keys().collect::<Vec<_>>(), ["core"]);
        assert!(!none.sections.contains_key(&Section::I));

        let other = coverage(&cat, &["welding".to_owned()], &BTreeMap::new());
        assert_eq!(other.overall.denominator, 2);

        let with = coverage(&cat, &["trades".to_owned()], &BTreeMap::new());
        assert_eq!(with.overall.denominator, 3);
        assert_eq!(with.packs.keys().collect::<Vec<_>>(), ["core", "trades"]);
        assert_eq!(with.packs["trades"].denominator, 1);
        assert_eq!(with.packs["core"].denominator, 2);
        assert_eq!(with.sections[&Section::I].denominator, 1);
    }

    #[test]
    fn optional_and_attestation_are_excluded() {
        let cat = mixed();
        let r = coverage(&cat, &[], &BTreeMap::new());
        assert_eq!(r.sections[&Section::C].denominator, 1);
        let ids: Vec<_> = r.top_missing.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["profile.name", "legal.a"]);
    }

    #[test]
    fn per_section_and_pack_tallies() {
        let cat = mixed();
        let st = states(&[
            ("legal.a", AnswerState::Answered),
            ("approach.trades.a", AnswerState::Declined),
        ]);
        let r = coverage(&cat, &["trades".to_owned()], &st);
        assert_eq!(r.sections[&Section::C].answered, 1);
        assert_eq!(r.sections[&Section::C].ratio, Some(1.0));
        assert_eq!(r.sections[&Section::A].missing, 1);
        assert_eq!(r.sections[&Section::A].ratio, Some(0.0));
        assert_eq!(r.sections[&Section::A].weighted_denominator, 2);
        assert_eq!(r.packs["trades"].declined, 1);
        assert_eq!(r.packs["core"].answered, 1);
        assert_eq!(r.packs["core"].ratio, Some(0.5));
        assert_eq!(r.overall.covered, 2);
    }

    #[test]
    fn suggested_is_not_covered_but_tallied_separately() {
        let cat = mixed();
        let r = coverage(&cat, &[], &states(&[("legal.a", AnswerState::Suggested)]));
        assert_eq!(r.overall.suggested, 1);
        assert_eq!(r.overall.covered, 0);
        assert_eq!(r.overall.denominator, 2);
        let entry = r.top_missing.iter().find(|m| m.id == "legal.a").unwrap();
        assert_eq!(entry.state, AnswerState::Suggested);
    }

    #[test]
    fn top_missing_orders_by_weight_then_id_and_caps_at_ten() {
        let mut body = String::new();
        for i in 0..12 {
            // Weights 1,2,3,1,2,3,...; ids sort lexically.
            body += &q(
                &format!("legal.q{i:02}"),
                "C",
                "legal",
                &format!("weight = {}", i % 3 + 1),
            );
        }
        let cat = Catalog::parse(&[("legal.toml", &body)]).unwrap();
        let st = states(&[("legal.q02", AnswerState::Answered)]);
        let r = coverage(&cat, &[], &st);
        let ids: Vec<_> = r.top_missing.iter().map(|m| m.id.as_str()).collect();
        // Weight 3 (q02 answered, so q05 q08 q11), weight 2 (q01 q04 q07 q10),
        // then weight 1 (q00 q03, capped at ten entries).
        assert_eq!(
            ids,
            [
                "legal.q05",
                "legal.q08",
                "legal.q11",
                "legal.q01",
                "legal.q04",
                "legal.q07",
                "legal.q10",
                "legal.q00",
                "legal.q03",
                "legal.q06"
            ]
        );
        assert_eq!(r.top_missing[0].weight, 3);
        assert_eq!(r.top_missing[0].section, Section::C);
    }

    /// `profile.bio` draws on `profile.skills.*`; `profile.plain` has no
    /// `draws_on`.
    fn composing() -> Catalog {
        let body = [
            q("profile.skills.go", "B", "contact", ""),
            q("profile.skills.rust", "B", "contact", ""),
            q(
                "profile.bio",
                "B",
                "free_text",
                "weight = 2\ndraws_on = ['profile.skills.*']",
            ),
            q("profile.plain", "B", "free_text", ""),
        ]
        .concat();
        Catalog::parse(&[("profile.toml", &body)]).unwrap()
    }

    #[test]
    fn composable_needs_an_answered_source() {
        let cat = composing();
        let bio = cat.get("profile.bio").unwrap();
        let plain = cat.get("profile.plain").unwrap();
        assert!(!composable(&cat, bio, &BTreeMap::new()));
        for unverified in [
            AnswerState::Suggested,
            AnswerState::Declined,
            AnswerState::NotApplicable,
            AnswerState::Missing,
        ] {
            let st = states(&[("profile.skills.go", unverified)]);
            assert!(!composable(&cat, bio, &st), "{unverified:?}");
        }
        let st = states(&[("profile.skills.rust", AnswerState::Answered)]);
        assert!(composable(&cat, bio, &st));
        assert!(!composable(&cat, plain, &st));
    }

    #[test]
    fn composable_is_covered_and_tallied_separately() {
        let cat = composing();
        let st = states(&[("profile.skills.go", AnswerState::Answered)]);
        let r = coverage(&cat, &[], &st);
        let t = &r.overall;
        assert_eq!((t.answered, t.composable, t.missing), (1, 1, 2));
        assert_eq!((t.covered, t.denominator), (2, 4));
        assert_eq!((t.weighted_covered, t.weighted_denominator), (3, 5));
        assert_eq!(r.sections[&Section::B].composable, 1);
        assert_eq!(r.packs["core"].composable, 1);
        let ids: Vec<_> = r.top_missing.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["profile.plain", "profile.skills.rust"]);
    }

    #[test]
    fn stored_or_policy_state_beats_composable() {
        let cat = composing();
        for own in [
            AnswerState::Suggested,
            AnswerState::Declined,
            AnswerState::AlwaysAsk,
            AnswerState::Answered,
        ] {
            let st = states(&[
                ("profile.skills.go", AnswerState::Answered),
                ("profile.bio", own),
            ]);
            let t = coverage(&cat, &[], &st).overall;
            assert_eq!(t.composable, 0, "{own:?}");
        }
    }

    #[test]
    fn report_serializes_with_section_keys() {
        let cat = mixed();
        let r = coverage(&cat, &[], &BTreeMap::new());
        let v = toml::Value::try_from(&r).unwrap();
        assert!(v["sections"]["A"].is_table());
        assert!(v["packs"]["core"].is_table());
        assert!(v["overall"]["ratio"].is_float());
        assert_eq!(v["top_missing"][0]["state"].as_str(), Some("missing"));
    }
}
