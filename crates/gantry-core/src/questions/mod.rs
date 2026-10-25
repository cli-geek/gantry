//! Question definitions (m2a question-bank design §1–§3, plan §7.3).
//!
//! A [`QuestionDef`] describes one canonical question: its address (the id),
//! its sensitivity class, the shape of its answer, and the phrasings seen on
//! real forms. The catalog is parsed and checked in [`catalog`]; free-text
//! enum answers are mapped to option keys in [`options`].
//!
//! Nothing here performs I/O and nothing here knows about any particular
//! occupation: pack questions arrive as data.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ats::Ats;

pub mod catalog;
pub mod coverage;
pub mod options;
pub mod value;

pub use catalog::{Catalog, CatalogError, CatalogErrors};
pub use coverage::{AnswerState, CoverageReport, MissingEntry, Stored, Tally, coverage, state_of};
pub use options::{OptionError, Resolved, resolve_option};
pub use value::{ValueError, ValueWarning, validate_value};

/// Question section A..K (design §1). Each section lives in one bank file.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum Section {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
}

impl Section {
    /// The bank file (and first id segment) this section's questions use.
    pub const fn file(self) -> &'static str {
        match self {
            Self::A | Self::B => "profile",
            Self::C | Self::D => "legal",
            Self::E => "eeo",
            Self::F => "availability",
            Self::G => "experience",
            Self::H => "stories",
            Self::I => "approach",
            Self::J | Self::K => "motivation",
        }
    }
}

/// Sensitivity classes (plan §7.3). `Attestation` and `Prohibited` questions
/// exist for classification only and are never addressable.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Contact,
    Legal,
    Eeo,
    Clearance,
    Availability,
    Numeric,
    Preference,
    FreeText,
    Attestation,
    Prohibited,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AnswerType {
    Bool,
    Enum,
    Date,
    Number,
    Text,
    List,
    File,
    Grid,
}

impl AnswerType {
    /// Types a list field may have: everything but `list` and `grid`.
    pub const fn is_scalar(self) -> bool {
        !matches!(self, Self::List | Self::Grid)
    }
}

/// The user's standing instruction for a question (design §1).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    Decline,
    AlwaysAsk,
    NotApplicable,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Unreviewed,
    Reviewed,
}

/// One column of an inline-record list (education, references, stories).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldDef {
    pub name: String,
    pub label: String,
    pub answer_type: AnswerType,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub optional: bool,
}

/// A phrasing of the question seen on a form, for tier-1a matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub text: String,
    pub ats: Ats,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestionDef {
    pub id: String,
    pub section: Section,
    pub label: String,
    pub sensitivity: Sensitivity,
    pub answer_type: AnswerType,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
    #[serde(default)]
    pub min_items: u32,
    #[serde(default)]
    pub variants: Vec<Variant>,
    /// Option key (or `yes`/`no`/`decline` for bool) to the form labels that
    /// mean it.
    #[serde(default)]
    pub options_normalization: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub jurisdiction_note: Option<String>,
    #[serde(default)]
    pub default_policy: Option<PolicyMode>,
    /// Optional questions are outside coverage.
    #[serde(default)]
    pub optional: bool,
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
}

impl QuestionDef {
    /// Attestation and prohibited questions have no address: `bank set`
    /// rejects them and coverage ignores them (design §2).
    pub fn is_addressable(&self) -> bool {
        !matches!(
            self.sensitivity,
            Sensitivity::Attestation | Sensitivity::Prohibited
        )
    }
}

/// Header of a pack file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub pack: String,
    pub review_status: ReviewStatus,
    #[serde(default)]
    pub reviewed_by: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_file_mapping() {
        let expect = [
            (Section::A, "profile"),
            (Section::B, "profile"),
            (Section::C, "legal"),
            (Section::D, "legal"),
            (Section::E, "eeo"),
            (Section::F, "availability"),
            (Section::G, "experience"),
            (Section::H, "stories"),
            (Section::I, "approach"),
            (Section::J, "motivation"),
            (Section::K, "motivation"),
        ];
        for (s, f) in expect {
            assert_eq!(s.file(), f);
        }
    }

    #[test]
    fn section_serializes_as_letter() {
        #[derive(Serialize, Deserialize)]
        struct W {
            s: Section,
        }
        let w: W = toml::from_str("s = \"K\"").unwrap();
        assert_eq!(w.s, Section::K);
        assert_eq!(toml::to_string(&w).unwrap().trim(), "s = \"K\"");
    }

    #[test]
    fn addressable_excludes_attestation_and_prohibited() {
        let mut d: QuestionDef = toml::from_str(
            "id='a.b'\nsection='A'\nlabel='x'\nsensitivity='contact'\nanswer_type='text'",
        )
        .unwrap();
        assert!(d.is_addressable());
        assert_eq!(d.weight, 1);
        d.sensitivity = Sensitivity::Attestation;
        assert!(!d.is_addressable());
        d.sensitivity = Sensitivity::Prohibited;
        assert!(!d.is_addressable());
    }
}
