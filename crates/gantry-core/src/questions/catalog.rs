//! Catalog parsing and load-time validation (design §1, §3).
//!
//! `Catalog::parse` takes the question files as `(file name, contents)`
//! pairs, e.g. `("legal.toml", ..)` or `("packs/<name>.toml", ..)`, and
//! reports every problem it finds rather than the first. Core files have no
//! header; pack files start with `pack`, `review_status` and `reviewed_by`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Deserialize;

use crate::text::normalize_label;

use super::{
    AnswerType, FieldDef, Pack, PolicyMode, QuestionDef, ReviewStatus, Section, Sensitivity,
    validate_value,
};

/// One broken rule. Every variant names the file; all but the file-level ones
/// also name the question id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("{file}: invalid TOML: {message}")]
    Toml { file: String, message: String },
    #[error("{file}: core file must not have a pack header")]
    CoreFileHeader { file: String },
    #[error("{file}: pack file needs a `pack` header")]
    PackHeaderMissing { file: String },
    #[error("{file}: pack name {pack:?} must equal the file stem {stem:?}")]
    PackNameMismatch {
        file: String,
        pack: String,
        stem: String,
    },
    #[error("{file}: pack name \"core\" is reserved for the core bank")]
    PackNameReserved { file: String },
    #[error("{file}: pack {pack:?} is also defined in {first_file}")]
    DuplicatePack {
        file: String,
        pack: String,
        first_file: String,
    },
    #[error("{file}: pack {pack:?} is reviewed but has no reviewed_by")]
    ReviewedWithoutReviewers { file: String, pack: String },
    #[error("{file}: pack {pack:?} is unreviewed but has reviewed_by")]
    UnreviewedWithReviewers { file: String, pack: String },

    #[error("{file}: {id}: id segment {segment:?} must match [a-z0-9_]+")]
    BadIdSegment {
        file: String,
        id: String,
        segment: String,
    },
    #[error("{file}: {id}: id needs at least two segments")]
    IdTooShort { file: String, id: String },
    #[error("{file}: {id}: id must start with {expected:?} for section {section:?}")]
    IdSectionMismatch {
        file: String,
        id: String,
        section: Section,
        expected: &'static str,
    },
    #[error("{file}: {id}: core file {file} may only hold {expected}.* ids")]
    IdFileMismatch {
        file: String,
        id: String,
        expected: String,
    },
    #[error("{file}: {id}: duplicate id (first in {first_file})")]
    DuplicateId {
        file: String,
        id: String,
        first_file: String,
    },
    #[error("{file}: {id}: pack question must be <file>.{pack}.<name>")]
    PackNamespace {
        file: String,
        id: String,
        pack: String,
    },
    #[error("{file}: {id}: second segment collides with pack {pack:?}")]
    CorePackCollision {
        file: String,
        id: String,
        pack: String,
    },

    #[error("{file}: {id}: label is empty")]
    LabelEmpty { file: String, id: String },
    #[error("{file}: {id}: weight must be at least 1")]
    WeightZero { file: String, id: String },
    #[error("{file}: {id}: options are only allowed on enum")]
    OptionsNotEnum { file: String, id: String },
    #[error("{file}: {id}: enum needs options")]
    EnumNoOptions { file: String, id: String },
    #[error("{file}: {id}: duplicate option {option:?}")]
    DuplicateOption {
        file: String,
        id: String,
        option: String,
    },
    #[error("{file}: {id}: enum needs options_normalization")]
    EnumNoNormalization { file: String, id: String },
    #[error("{file}: {id}: option {option:?} has no options_normalization entry")]
    OptionWithoutNormalization {
        file: String,
        id: String,
        option: String,
    },
    #[error("{file}: {id}: normalization key {key:?} is not in options")]
    NormalizationKeyNotOption {
        file: String,
        id: String,
        key: String,
    },
    #[error("{file}: {id}: bool normalization key {key:?} must be yes, no or decline")]
    BoolNormalizationKey {
        file: String,
        id: String,
        key: String,
    },
    #[error("{file}: {id}: bool normalization must include {key}")]
    BoolNormalizationMissing {
        file: String,
        id: String,
        key: &'static str,
    },
    #[error("{file}: {id}: options_normalization only applies to enum and bool")]
    NormalizationNotAllowed { file: String, id: String },

    #[error("{file}: {id}: fields only allowed on list")]
    FieldsNotList { file: String, id: String },
    #[error("{file}: {id}: min_items only allowed on list")]
    MinItemsNotList { file: String, id: String },
    #[error("{file}: {id}: field {field:?} must be a scalar type")]
    FieldNotScalar {
        file: String,
        id: String,
        field: String,
    },
    #[error("{file}: {id}: field name {field:?} must match [a-z0-9_]+")]
    FieldNameInvalid {
        file: String,
        id: String,
        field: String,
    },
    #[error("{file}: {id}: duplicate field {field:?}")]
    FieldNameDuplicate {
        file: String,
        id: String,
        field: String,
    },
    #[error("{file}: {id}: field {field:?} options are only allowed on enum")]
    FieldOptionsNotEnum {
        file: String,
        id: String,
        field: String,
    },
    #[error("{file}: {id}: enum field {field:?} needs options")]
    FieldEnumNoOptions {
        file: String,
        id: String,
        field: String,
    },
    #[error("{file}: {id}: grid is only allowed in section F")]
    GridSection { file: String, id: String },
    #[error("{file}: {id}: grid is only allowed in core files")]
    GridInPack { file: String, id: String },

    #[error("{file}: {id}: section D questions must be clearance")]
    SectionDSensitivity { file: String, id: String },
    #[error("{file}: {id}: section E questions must be eeo")]
    SectionESensitivity { file: String, id: String },
    #[error("{file}: {id}: section E questions must default to decline")]
    SectionEPolicy { file: String, id: String },
    #[error("{file}: {id}: attestation and prohibited questions take no default_policy")]
    PolicyOnUnaddressable { file: String, id: String },
    #[error("{file}: {id}: default_policy must not be not_applicable")]
    PolicyNotApplicable { file: String, id: String },

    #[error("{file}: {id}: variant text is empty")]
    VariantEmpty { file: String, id: String },
    #[error("{file}: {id}: duplicate variant {text:?}")]
    VariantDuplicate {
        file: String,
        id: String,
        text: String,
    },
    #[error("{file}: {id}: variant {text:?} is also under {other}")]
    VariantCollision {
        file: String,
        id: String,
        text: String,
        other: String,
    },

    #[error("{file}: {id}: intake_rank must be at least 1")]
    IntakeRankZero { file: String, id: String },
    #[error("{file}: {id}: intake_rank {rank} is also used by {other}")]
    IntakeRankDuplicate {
        file: String,
        id: String,
        rank: u32,
        other: String,
    },
    #[error("{file}: {id}: intake_rank is only allowed on addressable questions")]
    IntakeUnaddressable { file: String, id: String },
    #[error("{file}: {id}: intake_rank is not allowed with a default_policy")]
    IntakeWithPolicy { file: String, id: String },
    #[error("{file}: {id}: intake_rank is not allowed on optional questions")]
    IntakeOptional { file: String, id: String },

    #[error("{file}: {id}: derive source {from:?} does not exist")]
    DeriveSourceUnknown {
        file: String,
        id: String,
        from: String,
    },
    #[error("{file}: {id}: derive source {from:?} is not addressable")]
    DeriveSourceUnaddressable {
        file: String,
        id: String,
        from: String,
    },
    #[error("{file}: {id}: derive source must not be the question itself")]
    DeriveSelf { file: String, id: String },
    #[error("{file}: {id}: derive source {from:?} is {answer_type:?}; need bool, enum or number")]
    DeriveSourceType {
        file: String,
        id: String,
        from: String,
        answer_type: AnswerType,
    },
    #[error("{file}: {id}: derive equals for {from:?} is invalid: {message}")]
    DeriveEqualsInvalid {
        file: String,
        id: String,
        from: String,
        message: String,
    },
    #[error("{file}: {id}: derive value for {from:?} is invalid: {message}")]
    DeriveValueInvalid {
        file: String,
        id: String,
        from: String,
        message: String,
    },
    #[error("{file}: {id}: derive is only allowed on addressable questions")]
    DeriveTargetUnaddressable { file: String, id: String },
    #[error("{file}: {id}: derive is not allowed on list or grid questions")]
    DeriveTargetNotScalar { file: String, id: String },
    #[error("{file}: {id}: more than one derive rule for {from:?} equals {equals}")]
    DeriveDuplicate {
        file: String,
        id: String,
        from: String,
        equals: String,
    },

    #[error("{file}: {id}: draws_on is only allowed on text and list questions")]
    DrawsOnType { file: String, id: String },
    #[error("{file}: {id}: draws_on entry {entry:?} matches no question")]
    DrawsOnNoMatch {
        file: String,
        id: String,
        entry: String,
    },
    #[error("{file}: {id}: draws_on entry {entry:?} matches {matched}, which is {sensitivity:?}")]
    DrawsOnForbidden {
        file: String,
        id: String,
        entry: String,
        matched: String,
        sensitivity: Sensitivity,
    },
    #[error("{file}: {id}: draws_on entry {entry:?} is the question itself")]
    DrawsOnSelf {
        file: String,
        id: String,
        entry: String,
    },
    #[error("{file}: {id}: duplicate draws_on entry {entry:?}")]
    DrawsOnDuplicate {
        file: String,
        id: String,
        entry: String,
    },
}

/// Every problem found in one `Catalog::parse` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogErrors(pub Vec<CatalogError>);

impl fmt::Display for CatalogErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CatalogErrors {}

/// Shape of one question file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    pack: Option<String>,
    review_status: Option<ReviewStatus>,
    #[serde(default)]
    reviewed_by: Option<Vec<String>>,
    #[serde(default)]
    question: Vec<QuestionDef>,
}

/// A validated set of question definitions.
#[derive(Debug, Clone, PartialEq)]
pub struct Catalog {
    questions: BTreeMap<String, QuestionDef>,
    /// Owning pack per question id; `None` is core.
    owner: BTreeMap<String, Option<String>>,
    packs: BTreeMap<String, Pack>,
}

impl Catalog {
    pub fn parse(files: &[(&str, &str)]) -> Result<Catalog, CatalogErrors> {
        let mut errs = Vec::new();
        let mut questions: BTreeMap<String, QuestionDef> = BTreeMap::new();
        let mut owner: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut first_file: BTreeMap<String, String> = BTreeMap::new();
        let mut packs: BTreeMap<String, Pack> = BTreeMap::new();
        let mut pack_file: BTreeMap<String, String> = BTreeMap::new();
        // (file, definition) for the cross-file checks below.
        let mut all: Vec<(String, Option<String>, QuestionDef)> = Vec::new();

        for &(file, contents) in files {
            let raw: RawFile = match toml::from_str(contents) {
                Ok(r) => r,
                Err(e) => {
                    errs.push(CatalogError::Toml {
                        file: file.to_owned(),
                        message: e.message().to_owned(),
                    });
                    continue;
                }
            };
            let stem = file_stem(file);
            let pack_name = if file.starts_with("packs/") {
                check_pack_header(file, &stem, &raw, &mut packs, &mut pack_file, &mut errs);
                Some(stem.clone())
            } else {
                if raw.pack.is_some() || raw.review_status.is_some() || raw.reviewed_by.is_some() {
                    errs.push(CatalogError::CoreFileHeader {
                        file: file.to_owned(),
                    });
                }
                None
            };
            for def in raw.question {
                check_question(file, &stem, pack_name.as_deref(), &def, &mut errs);
                if let Some(first) = first_file.get(&def.id) {
                    errs.push(CatalogError::DuplicateId {
                        file: file.to_owned(),
                        id: def.id.clone(),
                        first_file: first.clone(),
                    });
                    continue;
                }
                first_file.insert(def.id.clone(), file.to_owned());
                all.push((file.to_owned(), pack_name.clone(), def));
            }
        }

        // Cross-file: core ids must not shadow a pack namespace.
        let mut seen_variants: BTreeMap<String, String> = BTreeMap::new();
        for (file, pack, def) in &all {
            if pack.is_none()
                && let Some(second) = def.id.split('.').nth(1)
                && packs.contains_key(second)
            {
                errs.push(CatalogError::CorePackCollision {
                    file: file.clone(),
                    id: def.id.clone(),
                    pack: second.to_owned(),
                });
            }
            for v in &def.variants {
                let norm = normalize_label(&v.text);
                if norm.is_empty() {
                    continue;
                }
                match seen_variants.get(&norm) {
                    // Same-id repeats are reported by `check_question`.
                    Some(other) if *other != def.id => {
                        errs.push(CatalogError::VariantCollision {
                            file: file.clone(),
                            id: def.id.clone(),
                            text: v.text.clone(),
                            other: other.clone(),
                        });
                    }
                    Some(_) => {}
                    None => {
                        seen_variants.insert(norm, def.id.clone());
                    }
                }
            }
        }

        check_cross_references(&all, &mut errs);

        if !errs.is_empty() {
            return Err(CatalogErrors(errs));
        }
        for (_, pack, def) in all {
            owner.insert(def.id.clone(), pack);
            questions.insert(def.id.clone(), def);
        }
        Ok(Catalog {
            questions,
            owner,
            packs,
        })
    }

    pub fn get(&self, id: &str) -> Option<&QuestionDef> {
        self.questions.get(id)
    }

    /// All questions in id order.
    pub fn iter(&self) -> impl Iterator<Item = &QuestionDef> {
        self.questions.values()
    }

    pub fn len(&self) -> usize {
        self.questions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }

    /// The pack a question belongs to; `None` for core questions and
    /// unknown ids.
    pub fn pack_of(&self, id: &str) -> Option<&str> {
        self.owner.get(id)?.as_deref()
    }

    pub fn pack(&self, name: &str) -> Option<&Pack> {
        self.packs.get(name)
    }

    pub fn packs(&self) -> impl Iterator<Item = &Pack> {
        self.packs.values()
    }

    /// The short intake set, lowest `intake_rank` first.
    pub fn intake(&self) -> Vec<&QuestionDef> {
        let mut out: Vec<&QuestionDef> = self
            .questions
            .values()
            .filter(|d| d.intake_rank.is_some())
            .collect();
        out.sort_by_key(|d| d.intake_rank);
        out
    }

    /// The questions `id`'s `draws_on` entries match, deduplicated, in id
    /// order. Empty for unknown ids.
    pub fn draws_on(&self, id: &str) -> Vec<&QuestionDef> {
        let Some(def) = self.questions.get(id) else {
            return Vec::new();
        };
        self.questions
            .values()
            .filter(|d| d.id != id && def.draws_on.iter().any(|e| entry_matches(e, &d.id)))
            .collect()
    }

    /// How many questions `id` feeds: `(derive_targets, draws_on_dependents)`
    /// are the questions with a derive rule from `id` and the questions whose
    /// `draws_on` matches `id`.
    pub fn fan_out(&self, id: &str) -> (usize, usize) {
        let targets = self
            .questions
            .values()
            .filter(|d| d.derive.iter().any(|r| r.from == id))
            .count();
        let dependents = self
            .questions
            .values()
            .filter(|d| d.id != id && d.draws_on.iter().any(|e| entry_matches(e, id)))
            .count();
        (targets, dependents)
    }
}

/// Whether a `draws_on` entry (exact id or `<prefix>.*`) matches `id`.
fn entry_matches(entry: &str, id: &str) -> bool {
    match entry.strip_suffix(".*") {
        Some(prefix) => id
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.')),
        None => entry == id,
    }
}

/// `legal.toml` → `legal`; `packs/tech.toml` → `tech`.
fn file_stem(file: &str) -> String {
    let base = file.rsplit('/').next().unwrap_or(file);
    base.strip_suffix(".toml").unwrap_or(base).to_owned()
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn check_pack_header(
    file: &str,
    stem: &str,
    raw: &RawFile,
    packs: &mut BTreeMap<String, Pack>,
    pack_file: &mut BTreeMap<String, String>,
    errs: &mut Vec<CatalogError>,
) {
    let Some(name) = raw.pack.clone() else {
        errs.push(CatalogError::PackHeaderMissing {
            file: file.to_owned(),
        });
        return;
    };
    // Coverage reports core questions under this key.
    if name == "core" {
        errs.push(CatalogError::PackNameReserved {
            file: file.to_owned(),
        });
    }
    if name != stem {
        errs.push(CatalogError::PackNameMismatch {
            file: file.to_owned(),
            pack: name.clone(),
            stem: stem.to_owned(),
        });
    }
    if let Some(first) = pack_file.get(&name) {
        errs.push(CatalogError::DuplicatePack {
            file: file.to_owned(),
            pack: name,
            first_file: first.clone(),
        });
        return;
    }
    let review_status = raw.review_status.unwrap_or(ReviewStatus::Unreviewed);
    let reviewed_by = raw.reviewed_by.clone().unwrap_or_default();
    match review_status {
        ReviewStatus::Reviewed if reviewed_by.is_empty() => {
            errs.push(CatalogError::ReviewedWithoutReviewers {
                file: file.to_owned(),
                pack: name.clone(),
            });
        }
        ReviewStatus::Unreviewed if !reviewed_by.is_empty() => {
            errs.push(CatalogError::UnreviewedWithReviewers {
                file: file.to_owned(),
                pack: name.clone(),
            });
        }
        _ => {}
    }
    pack_file.insert(name.clone(), file.to_owned());
    packs.insert(
        name.clone(),
        Pack {
            pack: name,
            review_status,
            reviewed_by,
        },
    );
}

/// Rules that need only the definition, its file and its pack.
fn check_question(
    file: &str,
    stem: &str,
    pack: Option<&str>,
    def: &QuestionDef,
    errs: &mut Vec<CatalogError>,
) {
    macro_rules! bad {
        ($v:ident $(, $k:ident : $val:expr)*) => {
            errs.push(CatalogError::$v {
                file: file.to_owned(),
                id: def.id.clone()
                $(, $k: $val)*
            })
        };
    }

    check_id(file, stem, pack, def, errs);

    if def.label.trim().is_empty() {
        bad!(LabelEmpty);
    }
    if def.weight < 1 {
        bad!(WeightZero);
    }

    let ty = def.answer_type;
    if ty != AnswerType::Enum && !def.options.is_empty() {
        bad!(OptionsNotEnum);
    }
    match ty {
        AnswerType::Enum => {
            if def.options.is_empty() {
                bad!(EnumNoOptions);
            }
            if let Some(dup) = first_duplicate(&def.options) {
                bad!(DuplicateOption, option: dup);
            }
            if def.options_normalization.is_empty() {
                bad!(EnumNoNormalization);
            } else {
                // The resolver matches keys only, so an option without an
                // entry could never be set.
                for option in &def.options {
                    if !def.options_normalization.contains_key(option) {
                        bad!(OptionWithoutNormalization, option: option.clone());
                    }
                }
            }
            for key in def.options_normalization.keys() {
                if !def.options.contains(key) {
                    bad!(NormalizationKeyNotOption, key: key.clone());
                }
            }
        }
        AnswerType::Bool => {
            let norm = &def.options_normalization;
            if !norm.is_empty() {
                for key in norm.keys() {
                    if !matches!(key.as_str(), "yes" | "no" | "decline") {
                        bad!(BoolNormalizationKey, key: key.clone());
                    }
                }
                for key in ["yes", "no"] {
                    if !norm.contains_key(key) {
                        bad!(BoolNormalizationMissing, key: key);
                    }
                }
            }
        }
        _ => {
            if !def.options_normalization.is_empty() {
                bad!(NormalizationNotAllowed);
            }
        }
    }

    if ty != AnswerType::List {
        if !def.fields.is_empty() {
            bad!(FieldsNotList);
        }
        if def.min_items > 0 {
            bad!(MinItemsNotList);
        }
    }
    check_fields(file, def, &def.fields, errs);

    if ty == AnswerType::Grid {
        if def.section != Section::F {
            bad!(GridSection);
        }
        if pack.is_some() {
            bad!(GridInPack);
        }
    }

    if def.section == Section::D && def.sensitivity != Sensitivity::Clearance {
        bad!(SectionDSensitivity);
    }
    if def.section == Section::E {
        if def.sensitivity != Sensitivity::Eeo {
            bad!(SectionESensitivity);
        }
        if def.default_policy != Some(PolicyMode::Decline) {
            bad!(SectionEPolicy);
        }
    }
    if def.default_policy.is_some() && !def.is_addressable() {
        bad!(PolicyOnUnaddressable);
    }
    if def.default_policy == Some(PolicyMode::NotApplicable) {
        bad!(PolicyNotApplicable);
    }

    let mut seen = BTreeSet::new();
    for v in &def.variants {
        let norm = normalize_label(&v.text);
        if norm.is_empty() {
            bad!(VariantEmpty);
        } else if !seen.insert(norm) {
            bad!(VariantDuplicate, text: v.text.clone());
        }
    }

    if let Some(rank) = def.intake_rank {
        if rank < 1 {
            bad!(IntakeRankZero);
        }
        if !def.is_addressable() {
            bad!(IntakeUnaddressable);
        }
        if def.default_policy.is_some() {
            bad!(IntakeWithPolicy);
        }
        if def.optional {
            bad!(IntakeOptional);
        }
    }

    if !def.draws_on.is_empty() && !matches!(ty, AnswerType::Text | AnswerType::List) {
        bad!(DrawsOnType);
    }
    let mut entries = BTreeSet::new();
    for entry in &def.draws_on {
        if !entries.insert(entry.as_str()) {
            bad!(DrawsOnDuplicate, entry: entry.clone());
        }
    }

    if !def.derive.is_empty() {
        if !def.is_addressable() {
            bad!(DeriveTargetUnaddressable);
        } else if !ty.is_scalar() {
            bad!(DeriveTargetNotScalar);
        }
    }
}

/// Rules that need every question's id: intake ranks, derive sources and
/// draws_on targets.
fn check_cross_references(
    all: &[(String, Option<String>, QuestionDef)],
    errs: &mut Vec<CatalogError>,
) {
    let by_id: BTreeMap<&str, &QuestionDef> =
        all.iter().map(|(_, _, d)| (d.id.as_str(), d)).collect();
    let mut ranks: BTreeMap<u32, &str> = BTreeMap::new();

    for (file, _, def) in all {
        macro_rules! bad {
            ($v:ident $(, $k:ident : $val:expr)*) => {
                errs.push(CatalogError::$v {
                    file: file.clone(),
                    id: def.id.clone()
                    $(, $k: $val)*
                })
            };
        }

        // Rank 0 is reported by `check_question`; it still counts as taken.
        if let Some(rank) = def.intake_rank {
            match ranks.get(&rank) {
                Some(other) => bad!(IntakeRankDuplicate, rank: rank, other: (*other).to_owned()),
                None => {
                    ranks.insert(rank, &def.id);
                }
            }
        }

        let mut seen_rules: Vec<(&str, &toml::Value)> = Vec::new();
        for rule in &def.derive {
            let from = rule.from.clone();
            if rule.from == def.id {
                bad!(DeriveSelf);
                continue;
            }
            if seen_rules
                .iter()
                .any(|(f, e)| *f == rule.from && super::derive::same(e, &rule.equals))
            {
                bad!(DeriveDuplicate, from: from.clone(), equals: rule.equals.to_string());
            }
            seen_rules.push((&rule.from, &rule.equals));

            let Some(source) = by_id.get(rule.from.as_str()) else {
                bad!(DeriveSourceUnknown, from: from.clone());
                continue;
            };
            if !source.is_addressable() {
                bad!(DeriveSourceUnaddressable, from: from.clone());
            } else if !matches!(
                source.answer_type,
                AnswerType::Bool | AnswerType::Enum | AnswerType::Number
            ) {
                bad!(DeriveSourceType, from: from.clone(), answer_type: source.answer_type);
            } else if let Err(e) = validate_value(source, &rule.equals) {
                bad!(DeriveEqualsInvalid, from: from.clone(), message: e.to_string());
            }
            if def.is_addressable()
                && def.answer_type.is_scalar()
                && let Err(e) = validate_value(def, &rule.value)
            {
                bad!(DeriveValueInvalid, from: from, message: e.to_string());
            }
        }

        for entry in &def.draws_on {
            if *entry == def.id {
                bad!(DrawsOnSelf, entry: entry.clone());
                continue;
            }
            // A prefix that happens to cover the question itself skips it.
            let matched: Vec<&&QuestionDef> = by_id
                .values()
                .filter(|d| d.id != def.id && entry_matches(entry, &d.id))
                .collect();
            if matched.is_empty() {
                bad!(DrawsOnNoMatch, entry: entry.clone());
            }
            for m in matched {
                if matches!(
                    m.sensitivity,
                    Sensitivity::Legal
                        | Sensitivity::Eeo
                        | Sensitivity::Clearance
                        | Sensitivity::Availability
                        | Sensitivity::Attestation
                        | Sensitivity::Prohibited
                ) {
                    bad!(
                        DrawsOnForbidden,
                        entry: entry.clone(),
                        matched: m.id.clone(),
                        sensitivity: m.sensitivity
                    );
                }
            }
        }
    }
}

fn check_id(
    file: &str,
    stem: &str,
    pack: Option<&str>,
    def: &QuestionDef,
    errs: &mut Vec<CatalogError>,
) {
    let (f, id) = (file.to_owned(), def.id.clone());
    let segments: Vec<&str> = def.id.split('.').collect();
    let mut valid = true;
    for s in &segments {
        if !is_ident(s) {
            valid = false;
            errs.push(CatalogError::BadIdSegment {
                file: f.clone(),
                id: id.clone(),
                segment: (*s).to_owned(),
            });
        }
    }
    if segments.len() < 2 {
        errs.push(CatalogError::IdTooShort { file: f, id });
        return;
    }
    if !valid {
        return;
    }
    let expected = def.section.file();
    if segments[0] != expected {
        errs.push(CatalogError::IdSectionMismatch {
            file: f.clone(),
            id: id.clone(),
            section: def.section,
            expected,
        });
    }
    match pack {
        None => {
            if segments[0] != stem {
                errs.push(CatalogError::IdFileMismatch {
                    file: f,
                    id,
                    expected: stem.to_owned(),
                });
            }
        }
        Some(p) => {
            if segments.len() < 3 || segments[1] != p {
                errs.push(CatalogError::PackNamespace {
                    file: f,
                    id,
                    pack: p.to_owned(),
                });
            }
        }
    }
}

fn check_fields(file: &str, def: &QuestionDef, fields: &[FieldDef], errs: &mut Vec<CatalogError>) {
    macro_rules! bad {
        ($v:ident, $field:expr) => {
            errs.push(CatalogError::$v {
                file: file.to_owned(),
                id: def.id.clone(),
                field: $field.name.clone(),
            })
        };
    }
    let mut names = BTreeSet::new();
    for fd in fields {
        if !is_ident(&fd.name) {
            bad!(FieldNameInvalid, fd);
        } else if !names.insert(fd.name.as_str()) {
            bad!(FieldNameDuplicate, fd);
        }
        if !fd.answer_type.is_scalar() {
            bad!(FieldNotScalar, fd);
        }
        match fd.answer_type {
            AnswerType::Enum => {
                if fd.options.is_empty() {
                    bad!(FieldEnumNoOptions, fd);
                }
                if let Some(dup) = first_duplicate(&fd.options) {
                    errs.push(CatalogError::DuplicateOption {
                        file: file.to_owned(),
                        id: def.id.clone(),
                        option: dup,
                    });
                }
            }
            _ if !fd.options.is_empty() => bad!(FieldOptionsNotEnum, fd),
            _ => {}
        }
    }
}

fn first_duplicate(items: &[String]) -> Option<String> {
    let mut seen = BTreeSet::new();
    items.iter().find(|o| !seen.insert(o.as_str())).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `[[question]]` table; `extra` is appended as more keys (use inline
    /// tables for maps).
    fn q(id: &str, section: &str, sens: &str, ty: &str, extra: &str) -> String {
        format!(
            "[[question]]\nid = '{id}'\nsection = '{section}'\nlabel = 'L'\n\
             sensitivity = '{sens}'\nanswer_type = '{ty}'\n{extra}\n"
        )
    }

    /// A plain valid text question in legal.toml.
    fn legal(name: &str, extra: &str) -> String {
        q(&format!("legal.{name}"), "C", "legal", "text", extra)
    }

    fn parse1(file: &str, body: &str) -> Result<Catalog, CatalogErrors> {
        Catalog::parse(&[(file, body)])
    }

    fn errors(file: &str, body: &str) -> Vec<CatalogError> {
        parse1(file, body).expect_err("expected errors").0
    }

    fn pack_header(name: &str) -> String {
        format!("pack = '{name}'\nreview_status = 'unreviewed'\nreviewed_by = []\n")
    }

    /// Asserts `body` in `file` fails with an error matching `$pat`, and that
    /// `good` passes.
    macro_rules! rule {
        ($file:expr, $good:expr, $bad:expr, $pat:pat) => {{
            let good: String = $good;
            parse1($file, &good).unwrap_or_else(|e| panic!("good case failed: {e}"));
            let bad: String = $bad;
            let errs = errors($file, &bad);
            assert!(
                errs.iter().any(|e| matches!(e, $pat)),
                "no matching error in {errs:#?}"
            );
        }};
    }

    #[test]
    fn parses_and_accessors() {
        let cat = Catalog::parse(&[
            ("legal.toml", &legal("a", "")),
            (
                "packs/tech.toml",
                &format!(
                    "{}{}",
                    pack_header("tech"),
                    q("approach.tech.outage", "I", "free_text", "text", "")
                ),
            ),
        ])
        .unwrap();
        assert_eq!(cat.len(), 2);
        assert!(cat.get("legal.a").is_some());
        assert_eq!(cat.pack_of("legal.a"), None);
        assert_eq!(cat.pack_of("approach.tech.outage"), Some("tech"));
        assert_eq!(
            cat.pack("tech").unwrap().review_status,
            ReviewStatus::Unreviewed
        );
        assert_eq!(cat.iter().count(), 2);
    }

    #[test]
    fn toml_syntax_error() {
        let errs = errors("legal.toml", "[[question]\n");
        assert!(matches!(&errs[..], [CatalogError::Toml { file, .. }] if file == "legal.toml"));
    }

    #[test]
    fn unknown_field_is_a_toml_error() {
        let errs = errors("legal.toml", &legal("a", "bogus = 1"));
        assert!(matches!(&errs[..], [CatalogError::Toml { .. }]));
    }

    #[test]
    fn collects_errors_across_files() {
        let errs = Catalog::parse(&[
            ("legal.toml", "[[question]\n"),
            ("eeo.toml", &legal("a", "")),
            (
                "profile.toml",
                &q("profile.Bad", "A", "contact", "text", ""),
            ),
        ])
        .unwrap_err()
        .0;
        assert!(errs.iter().any(|e| matches!(e, CatalogError::Toml { .. })));
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::IdFileMismatch { .. }))
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::BadIdSegment { .. }))
        );
    }

    #[test]
    fn collects_several_errors_in_one_question() {
        let body = q(
            "legal.x",
            "C",
            "legal",
            "text",
            "options = ['a']\nweight = 0",
        );
        let errs = errors("legal.toml", &body);
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::OptionsNotEnum { .. }))
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::WeightZero { .. }))
        );
    }

    #[test]
    fn id_segments() {
        rule!(
            "legal.toml",
            legal("a_1.b", ""),
            legal("Bad", ""),
            CatalogError::BadIdSegment { .. }
        );
        rule!(
            "legal.toml",
            legal("a", ""),
            legal("a..b", ""),
            CatalogError::BadIdSegment { .. }
        );
    }

    #[test]
    fn id_needs_two_segments() {
        rule!(
            "legal.toml",
            legal("a", ""),
            q("legal", "C", "legal", "text", ""),
            CatalogError::IdTooShort { .. }
        );
    }

    #[test]
    fn id_first_segment_matches_section() {
        rule!(
            "legal.toml",
            legal("a", ""),
            q("legal.a", "A", "contact", "text", ""),
            CatalogError::IdSectionMismatch { .. }
        );
    }

    #[test]
    fn core_file_stem_matches_first_segment() {
        rule!(
            "legal.toml",
            legal("a", ""),
            q("profile.a", "A", "contact", "text", ""),
            CatalogError::IdFileMismatch { .. }
        );
    }

    #[test]
    fn duplicate_ids_across_files() {
        let both = format!("{}{}", legal("a", ""), legal("a", ""));
        let errs = errors("legal.toml", &both);
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::DuplicateId { .. }))
        );
        let errs = Catalog::parse(&[
            ("legal.toml", &legal("a", "")),
            ("legal.toml", &legal("a", "")),
        ])
        .unwrap_err()
        .0;
        assert!(
            matches!(&errs[..], [CatalogError::DuplicateId { first_file, .. }] if first_file == "legal.toml")
        );
    }

    #[test]
    fn core_file_rejects_header() {
        rule!(
            "legal.toml",
            legal("a", ""),
            format!("pack = 'legal'\n{}", legal("a", "")),
            CatalogError::CoreFileHeader { .. }
        );
    }

    #[test]
    fn pack_header_required_and_named_after_file() {
        let body = q("approach.tech.a", "I", "free_text", "text", "");
        rule!(
            "packs/tech.toml",
            format!("{}{body}", pack_header("tech")),
            body.clone(),
            CatalogError::PackHeaderMissing { .. }
        );
        rule!(
            "packs/tech.toml",
            format!("{}{body}", pack_header("tech")),
            format!("{}{body}", pack_header("other")),
            CatalogError::PackNameMismatch { .. }
        );
        let core_body = q("approach.core.a", "I", "free_text", "text", "");
        let errs = errors(
            "packs/core.toml",
            &format!("{}{core_body}", pack_header("core")),
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::PackNameReserved { .. })),
            "{errs:?}"
        );
    }

    #[test]
    fn pack_question_namespace() {
        rule!(
            "packs/tech.toml",
            format!(
                "{}{}",
                pack_header("tech"),
                q("approach.tech.a", "I", "free_text", "text", "")
            ),
            format!(
                "{}{}",
                pack_header("tech"),
                q("approach.other.a", "I", "free_text", "text", "")
            ),
            CatalogError::PackNamespace { .. }
        );
    }

    #[test]
    fn core_question_may_not_use_pack_name() {
        let pack = format!(
            "{}{}",
            pack_header("tech"),
            q("approach.tech.a", "I", "free_text", "text", "")
        );
        let ok = Catalog::parse(&[
            (
                "approach.toml",
                &q("approach.style.a", "I", "free_text", "text", ""),
            ),
            ("packs/tech.toml", &pack),
        ]);
        assert!(ok.is_ok());
        let errs = Catalog::parse(&[
            (
                "approach.toml",
                &q("approach.tech.b", "I", "free_text", "text", ""),
            ),
            ("packs/tech.toml", &pack),
        ])
        .unwrap_err()
        .0;
        assert!(
            errs.iter().any(
                |e| matches!(e, CatalogError::CorePackCollision { pack, .. } if pack == "tech")
            )
        );
    }

    #[test]
    fn review_status_consistency() {
        let body = q("approach.tech.a", "I", "free_text", "text", "");
        let hdr = |status: &str, by: &str| {
            format!("pack = 'tech'\nreview_status = '{status}'\nreviewed_by = {by}\n{body}")
        };
        rule!(
            "packs/tech.toml",
            hdr("reviewed", "['me']"),
            hdr("reviewed", "[]"),
            CatalogError::ReviewedWithoutReviewers { .. }
        );
        rule!(
            "packs/tech.toml",
            hdr("unreviewed", "[]"),
            hdr("unreviewed", "['me']"),
            CatalogError::UnreviewedWithReviewers { .. }
        );
    }

    #[test]
    fn same_pack_in_two_files() {
        let body = format!(
            "{}{}",
            pack_header("tech"),
            q("approach.tech.a", "I", "free_text", "text", "")
        );
        let errs = Catalog::parse(&[("packs/tech.toml", &body), ("packs/tech2.toml", &body)])
            .unwrap_err()
            .0;
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::DuplicatePack { .. }))
        );
    }

    #[test]
    fn options_only_on_enum() {
        rule!(
            "legal.toml",
            legal("a", ""),
            legal("a", "options = ['x']"),
            CatalogError::OptionsNotEnum { .. }
        );
    }

    #[test]
    fn enum_rules() {
        let en = |extra: &str| q("legal.e", "C", "legal", "enum", extra);
        let good = en("options = ['a','b']\noptions_normalization = { a = ['A'], b = ['B'] }");
        parse1("legal.toml", &good).unwrap();
        rule!(
            "legal.toml",
            good.clone(),
            en("options_normalization = { a = ['A'] }"),
            CatalogError::EnumNoOptions { .. }
        );
        rule!(
            "legal.toml",
            good.clone(),
            en("options = ['a','b']"),
            CatalogError::EnumNoNormalization { .. }
        );
        rule!(
            "legal.toml",
            good.clone(),
            en("options = ['a','b']\noptions_normalization = { a = ['A'], b = ['B'], c = ['C'] }"),
            CatalogError::NormalizationKeyNotOption { .. }
        );
        rule!(
            "legal.toml",
            good.clone(),
            en("options = ['a','b']\noptions_normalization = { a = ['A'] }"),
            CatalogError::OptionWithoutNormalization { .. }
        );
        rule!(
            "legal.toml",
            good,
            en("options = ['a','a']\noptions_normalization = { a = ['A'] }"),
            CatalogError::DuplicateOption { .. }
        );
    }

    #[test]
    fn bool_normalization_rules() {
        let b = |extra: &str| q("legal.b", "C", "legal", "bool", extra);
        parse1("legal.toml", &b("")).unwrap();
        let good = b("options_normalization = { yes = ['Y'], no = ['N'], decline = ['D'] }");
        parse1("legal.toml", &good).unwrap();
        rule!(
            "legal.toml",
            good.clone(),
            b("options_normalization = { yes = ['Y'], no = ['N'], maybe = ['M'] }"),
            CatalogError::BoolNormalizationKey { .. }
        );
        rule!(
            "legal.toml",
            good,
            b("options_normalization = { yes = ['Y'] }"),
            CatalogError::BoolNormalizationMissing { key: "no", .. }
        );
    }

    #[test]
    fn normalization_rejected_on_other_types() {
        rule!(
            "legal.toml",
            legal("a", ""),
            legal("a", "options_normalization = { a = ['A'] }"),
            CatalogError::NormalizationNotAllowed { .. }
        );
    }

    #[test]
    fn fields_and_min_items_only_on_list() {
        let fld = "fields = [{ name = 'school', label = 'School', answer_type = 'text' }]";
        let list = |extra: &str| q("profile.edu", "B", "contact", "list", extra);
        parse1("profile.toml", &list(&format!("{fld}\nmin_items = 1"))).unwrap();
        let text = |extra: &str| q("profile.edu", "B", "contact", "text", extra);
        rule!(
            "profile.toml",
            list(fld),
            text(fld),
            CatalogError::FieldsNotList { .. }
        );
        rule!(
            "profile.toml",
            list("min_items = 1"),
            text("min_items = 1"),
            CatalogError::MinItemsNotList { .. }
        );
    }

    #[test]
    fn field_rules() {
        let list = |fld: &str| {
            q(
                "profile.edu",
                "B",
                "contact",
                "list",
                &format!("fields = [{fld}]"),
            )
        };
        let ok = "{ name = 'school', label = 'S', answer_type = 'text' }";
        parse1("profile.toml", &list(ok)).unwrap();
        let enum_ok = "{ name = 'lvl', label = 'S', answer_type = 'enum', options = ['a'] }";
        parse1("profile.toml", &list(enum_ok)).unwrap();
        rule!(
            "profile.toml",
            list(ok),
            list("{ name = 'school', label = 'S', answer_type = 'list' }"),
            CatalogError::FieldNotScalar { .. }
        );
        rule!(
            "profile.toml",
            list(ok),
            list("{ name = 'School', label = 'S', answer_type = 'text' }"),
            CatalogError::FieldNameInvalid { .. }
        );
        rule!(
            "profile.toml",
            list(ok),
            list(&format!("{ok}, {ok}")),
            CatalogError::FieldNameDuplicate { .. }
        );
        rule!(
            "profile.toml",
            list(enum_ok),
            list("{ name = 'lvl', label = 'S', answer_type = 'enum' }"),
            CatalogError::FieldEnumNoOptions { .. }
        );
        rule!(
            "profile.toml",
            list(ok),
            list("{ name = 'school', label = 'S', answer_type = 'text', options = ['a'] }"),
            CatalogError::FieldOptionsNotEnum { .. }
        );
    }

    #[test]
    fn grid_only_in_section_f_core() {
        let grid = |sec: &str, id: &str| q(id, sec, "availability", "grid", "");
        rule!(
            "availability.toml",
            grid("F", "availability.hours"),
            grid("A", "profile.hours"),
            CatalogError::GridSection { .. }
        );
        let bad = format!(
            "{}{}",
            pack_header("tech"),
            grid("F", "availability.tech.hours")
        );
        let errs = errors("packs/tech.toml", &bad);
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::GridInPack { .. }))
        );
    }

    #[test]
    fn section_d_must_be_clearance() {
        rule!(
            "legal.toml",
            q("legal.cl", "D", "clearance", "bool", ""),
            q("legal.cl", "D", "legal", "bool", ""),
            CatalogError::SectionDSensitivity { .. }
        );
    }

    #[test]
    fn section_e_must_be_eeo_and_decline() {
        let e = |sens: &str, extra: &str| q("eeo.gender", "E", sens, "text", extra);
        let good = e("eeo", "default_policy = 'decline'");
        parse1("eeo.toml", &good).unwrap();
        rule!(
            "eeo.toml",
            good.clone(),
            e("legal", "default_policy = 'decline'"),
            CatalogError::SectionESensitivity { .. }
        );
        rule!(
            "eeo.toml",
            good,
            e("eeo", ""),
            CatalogError::SectionEPolicy { .. }
        );
    }

    #[test]
    fn default_policy_rules() {
        rule!(
            "legal.toml",
            legal("a", "default_policy = 'always_ask'"),
            q(
                "legal.a",
                "C",
                "attestation",
                "bool",
                "default_policy = 'decline'"
            ),
            CatalogError::PolicyOnUnaddressable { .. }
        );
        let errs = errors(
            "legal.toml",
            &q(
                "legal.a",
                "C",
                "prohibited",
                "text",
                "default_policy = 'always_ask'",
            ),
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, CatalogError::PolicyOnUnaddressable { .. }))
        );
        rule!(
            "legal.toml",
            legal("a", "default_policy = 'decline'"),
            legal("a", "default_policy = 'not_applicable'"),
            CatalogError::PolicyNotApplicable { .. }
        );
        parse1("legal.toml", &q("legal.a", "C", "attestation", "bool", "")).unwrap();
    }

    #[test]
    fn variant_rules() {
        let v = |t: &str| format!("variants = [{{ text = '{t}', ats = 'greenhouse' }}]");
        rule!(
            "legal.toml",
            legal("a", &v("Are you 18?")),
            legal("a", &v("  *")),
            CatalogError::VariantEmpty { .. }
        );
        rule!(
            "legal.toml",
            legal("a", &v("Are you 18?")),
            legal(
                "a",
                "variants = [{ text = 'Are you 18?', ats = 'lever' }, \
                 { text = 'ARE  you 18? *', ats = 'ashby' }]"
            ),
            CatalogError::VariantDuplicate { .. }
        );
    }

    #[test]
    fn variant_collision_across_ids_names_both() {
        let v = |t: &str| format!("variants = [{{ text = '{t}', ats = 'greenhouse' }}]");
        let good = format!("{}{}", legal("a", &v("One")), legal("b", &v("Two")));
        parse1("legal.toml", &good).unwrap();
        let bad = format!(
            "{}{}",
            legal("a", &v("Are you 18?")),
            legal("b", &v("are you 18? *"))
        );
        let errs = errors("legal.toml", &bad);
        assert!(errs.iter().any(|e| matches!(e,
            CatalogError::VariantCollision { id, other, .. } if id == "legal.b" && other == "legal.a")));
    }

    #[test]
    fn variant_ats_must_be_known() {
        let errs = errors(
            "legal.toml",
            &legal("a", "variants = [{ text = 'x', ats = 'taleo' }]"),
        );
        assert!(matches!(&errs[..], [CatalogError::Toml { .. }]));
    }

    #[test]
    fn weight_and_label() {
        rule!(
            "legal.toml",
            legal("a", "weight = 3"),
            legal("a", "weight = 0"),
            CatalogError::WeightZero { .. }
        );
        rule!(
            "legal.toml",
            legal("a", ""),
            legal("a", "").replace("label = 'L'", "label = ' '"),
            CatalogError::LabelEmpty { .. }
        );
    }

    #[test]
    fn error_display_is_short_and_specific() {
        let errs = errors("legal.toml", &legal("a", "weight = 0"));
        assert_eq!(
            errs[0].to_string(),
            "legal.toml: legal.a: weight must be at least 1"
        );
        assert_eq!(
            CatalogErrors(errs).to_string(),
            "legal.toml: legal.a: weight must be at least 1"
        );
    }

    /// legal.toml with a bool source `legal.src` plus `body`.
    fn with_src(body: &str) -> String {
        format!("{}{body}", q("legal.src", "C", "legal", "bool", ""))
    }

    fn rule_on(extra: &str) -> String {
        with_src(&legal("t", extra))
    }

    #[test]
    fn intake_rank_rules() {
        rule!(
            "legal.toml",
            legal("a", "intake_rank = 1"),
            legal("a", "intake_rank = 0"),
            CatalogError::IntakeRankZero { .. }
        );
        rule!(
            "legal.toml",
            format!(
                "{}{}",
                legal("a", "intake_rank = 1"),
                legal("b", "intake_rank = 2")
            ),
            format!(
                "{}{}",
                legal("a", "intake_rank = 1"),
                legal("b", "intake_rank = 1")
            ),
            CatalogError::IntakeRankDuplicate { .. }
        );
        let errs = errors(
            "legal.toml",
            &format!(
                "{}{}",
                legal("a", "intake_rank = 1"),
                legal("b", "intake_rank = 1")
            ),
        );
        assert!(matches!(
            &errs[..],
            [CatalogError::IntakeRankDuplicate { id, other, rank: 1, .. }]
                if id == "legal.b" && other == "legal.a"
        ));
        rule!(
            "legal.toml",
            legal("a", "intake_rank = 1"),
            q("legal.a", "C", "attestation", "text", "intake_rank = 1"),
            CatalogError::IntakeUnaddressable { .. }
        );
        rule!(
            "legal.toml",
            legal("a", "intake_rank = 1"),
            legal("a", "intake_rank = 1\ndefault_policy = 'always_ask'"),
            CatalogError::IntakeWithPolicy { .. }
        );
        rule!(
            "legal.toml",
            legal("a", "intake_rank = 1"),
            legal("a", "intake_rank = 1\noptional = true"),
            CatalogError::IntakeOptional { .. }
        );
    }

    #[test]
    fn intake_is_sorted_by_rank() {
        let body = format!(
            "{}{}{}",
            legal("a", "intake_rank = 3"),
            legal("b", "intake_rank = 1"),
            legal("c", "")
        );
        let cat = parse1("legal.toml", &body).unwrap();
        let ids: Vec<&str> = cat.intake().iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["legal.b", "legal.a"]);
    }

    const RULE: &str = "derive = [{ from = 'legal.src', equals = true, value = 'x' }]";

    #[test]
    fn derive_source_must_exist() {
        rule!(
            "legal.toml",
            rule_on(RULE),
            legal(
                "t",
                "derive = [{ from = 'legal.nope', equals = true, value = 'x' }]"
            ),
            CatalogError::DeriveSourceUnknown { .. }
        );
    }

    #[test]
    fn derive_source_must_be_addressable() {
        let att = q("legal.src", "C", "attestation", "bool", "");
        rule!(
            "legal.toml",
            rule_on(RULE),
            format!("{att}{}", legal("t", RULE)),
            CatalogError::DeriveSourceUnaddressable { .. }
        );
    }

    #[test]
    fn derive_source_is_not_self() {
        rule!(
            "legal.toml",
            rule_on(RULE),
            legal(
                "t",
                "derive = [{ from = 'legal.t', equals = true, value = 'x' }]"
            ),
            CatalogError::DeriveSelf { .. }
        );
    }

    #[test]
    fn derive_source_type() {
        let text_src = q("legal.src", "C", "legal", "text", "");
        rule!(
            "legal.toml",
            rule_on(RULE),
            format!(
                "{text_src}{}",
                legal(
                    "t",
                    "derive = [{ from = 'legal.src', equals = 'a', value = 'x' }]"
                )
            ),
            CatalogError::DeriveSourceType {
                answer_type: AnswerType::Text,
                ..
            }
        );
    }

    #[test]
    fn derive_equals_must_fit_source() {
        rule!(
            "legal.toml",
            rule_on(RULE),
            rule_on("derive = [{ from = 'legal.src', equals = 'yes', value = 'x' }]"),
            CatalogError::DeriveEqualsInvalid { .. }
        );
    }

    #[test]
    fn derive_value_must_fit_target() {
        let bool_t = |v: &str| {
            with_src(&q(
                "legal.t",
                "C",
                "legal",
                "bool",
                &format!("derive = [{{ from = 'legal.src', equals = true, value = {v} }}]"),
            ))
        };
        rule!(
            "legal.toml",
            bool_t("false"),
            bool_t("'x'"),
            CatalogError::DeriveValueInvalid { .. }
        );
    }

    #[test]
    fn derive_target_must_be_addressable_and_scalar() {
        let t = |sens: &str, ty: &str| {
            with_src(&q(
                "legal.t",
                "C",
                sens,
                ty,
                "derive = [{ from = 'legal.src', equals = true, value = true }]",
            ))
        };
        rule!(
            "legal.toml",
            t("legal", "bool"),
            t("attestation", "bool"),
            CatalogError::DeriveTargetUnaddressable { .. }
        );
        rule!(
            "legal.toml",
            t("legal", "bool"),
            t("legal", "list"),
            CatalogError::DeriveTargetNotScalar { .. }
        );
    }

    #[test]
    fn derive_duplicate_rule() {
        let two = |second: &str| {
            rule_on(&format!(
                "derive = [{{ from = 'legal.src', equals = true, value = 'x' }}, \
                 {{ from = 'legal.src', equals = {second}, value = 'y' }}]"
            ))
        };
        rule!(
            "legal.toml",
            two("false"),
            two("true"),
            CatalogError::DeriveDuplicate { .. }
        );
    }

    #[test]
    fn derive_rule_rejects_unknown_keys() {
        let errs = errors(
            "legal.toml",
            &rule_on("derive = [{ from = 'legal.src', equals = true, value = 'x', extra = 1 }]"),
        );
        assert!(matches!(&errs[..], [CatalogError::Toml { .. }]));
    }

    #[test]
    fn draws_on_only_text_or_list() {
        let t = |ty: &str| {
            format!(
                "{}{}",
                q("profile.f", "A", "contact", "text", ""),
                q("profile.t", "A", "contact", ty, "draws_on = ['profile.f']")
            )
        };
        rule!(
            "profile.toml",
            t("text"),
            t("bool"),
            CatalogError::DrawsOnType { .. }
        );
        parse1("profile.toml", &t("list")).unwrap();
    }

    #[test]
    fn draws_on_must_match() {
        let pf = q("profile.f", "A", "contact", "text", "");
        let t = |entry: &str| {
            format!(
                "{pf}{}",
                q(
                    "profile.t",
                    "A",
                    "contact",
                    "text",
                    &format!("draws_on = ['{entry}']")
                )
            )
        };
        rule!(
            "profile.toml",
            t("profile.f"),
            t("profile.g"),
            CatalogError::DrawsOnNoMatch { .. }
        );
        rule!(
            "profile.toml",
            t("profile.*"),
            t("nope.*"),
            CatalogError::DrawsOnNoMatch { .. }
        );
        // A prefix matches whole segments only.
        rule!(
            "profile.toml",
            t("profile.*"),
            t("prof.*"),
            CatalogError::DrawsOnNoMatch { .. }
        );
    }

    #[test]
    fn draws_on_refuses_sensitive_matches() {
        let t = |sens: &str, entry: &str| {
            format!(
                "{}{}",
                q("profile.f", "A", sens, "text", ""),
                q(
                    "profile.t",
                    "A",
                    "contact",
                    "text",
                    &format!("draws_on = ['{entry}']")
                )
            )
        };
        for sens in ["contact", "numeric", "preference", "free_text"] {
            parse1("profile.toml", &t(sens, "profile.f")).unwrap();
        }
        for sens in [
            "legal",
            "eeo",
            "clearance",
            "availability",
            "attestation",
            "prohibited",
        ] {
            // Exact entry and prefix entry both reach the question.
            for entry in ["profile.f", "profile.*"] {
                let errs = errors("profile.toml", &t(sens, entry));
                assert!(
                    errs.iter().any(|e| matches!(
                        e,
                        CatalogError::DrawsOnForbidden { matched, .. } if matched == "profile.f"
                    )),
                    "{sens} {entry}: {errs:#?}"
                );
            }
        }
    }

    #[test]
    fn draws_on_not_self_and_not_duplicate() {
        let pf = q("profile.f", "A", "contact", "text", "");
        let t = |extra: &str| format!("{pf}{}", q("profile.t", "A", "contact", "text", extra));
        rule!(
            "profile.toml",
            t("draws_on = ['profile.f']"),
            t("draws_on = ['profile.t']"),
            CatalogError::DrawsOnSelf { .. }
        );
        rule!(
            "profile.toml",
            t("draws_on = ['profile.f']"),
            t("draws_on = ['profile.f', 'profile.f']"),
            CatalogError::DrawsOnDuplicate { .. }
        );
    }

    #[test]
    fn draws_on_resolution_and_fan_out() {
        let body = format!(
            "{}{}{}{}{}",
            q("profile.skills.rust", "A", "contact", "text", ""),
            q("profile.skills.go", "A", "contact", "text", ""),
            q("profile.skillset", "A", "contact", "text", ""),
            q(
                "profile.summary",
                "A",
                "contact",
                "text",
                "draws_on = ['profile.skills.*', 'profile.skills.go', 'profile.skillset']"
            ),
            q(
                "profile.bio",
                "A",
                "contact",
                "text",
                "draws_on = ['profile.skills.*']"
            )
        );
        let cat = parse1("profile.toml", &body).unwrap();
        let ids = |v: Vec<&QuestionDef>| v.iter().map(|d| d.id.clone()).collect::<Vec<_>>();
        assert_eq!(
            ids(cat.draws_on("profile.summary")),
            [
                "profile.skills.go",
                "profile.skills.rust",
                "profile.skillset"
            ]
        );
        assert_eq!(
            ids(cat.draws_on("profile.bio")),
            ["profile.skills.go", "profile.skills.rust"]
        );
        assert!(cat.draws_on("profile.skillset").is_empty());
        assert!(cat.draws_on("profile.nope").is_empty());
        assert_eq!(cat.fan_out("profile.skills.go"), (0, 2));
        assert_eq!(cat.fan_out("profile.skillset"), (0, 1));
        assert_eq!(cat.fan_out("profile.summary"), (0, 0));
    }

    #[test]
    fn fan_out_counts_derive_targets() {
        let body = format!(
            "{}{}{}{}",
            q("legal.src", "C", "legal", "bool", ""),
            q(
                "legal.a",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.src', equals = true, value = true }]"
            ),
            q(
                "legal.b",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.src', equals = true, value = true }, { from = 'legal.src', equals = false, value = false }]"
            ),
            q("legal.c", "C", "legal", "bool", "")
        );
        let cat = parse1("legal.toml", &body).unwrap();
        assert_eq!(cat.fan_out("legal.src"), (2, 0));
        assert_eq!(cat.fan_out("legal.c"), (0, 0));
    }
}
