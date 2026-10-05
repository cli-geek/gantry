//! The search profile, stored in `search.toml` (§7.2).
//!
//! Every section is optional and defaults to "any": an empty profile
//! filters nothing. No occupation, title, location or pay value is ever
//! defaulted in code. Unknown keys are rejected so a typo in a hand-edited
//! file fails loudly instead of silently disabling a filter.
//!
//! M1 covers the sections the deterministic filters use. Benefits,
//! schedule, physical requirements, languages and scoring priorities arrive
//! with onboarding (M2) and scoring (M3).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geo::Distance;
use crate::{JobType, PayPeriod, WorkMode};

/// How a section affects a posting: exclude it, only weight its score, or
/// ignore the section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Hard,
    Prefer,
    #[default]
    Any,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SearchProfile {
    pub schema_version: u32,
    /// Occupation pack ids (e.g. "technology"). Selects question packs and
    /// the bundled company lists to poll.
    pub occupations: Vec<String>,
    pub titles: Titles,
    pub seniority: Seniority,
    pub experience: Experience,
    pub job_type: JobTypes,
    pub location: LocationPrefs,
    pub pay: PayPrefs,
    pub freshness: Freshness,
    pub keywords: Keywords,
    pub companies: Companies,
    pub requirements: Requirements,
    pub volume: Volume,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Titles {
    pub mode: Mode,
    pub targets: Vec<String>,
    /// Suggested at onboarding; only `verified` ones are used (invariant 3).
    pub synonyms: Vec<Synonym>,
    /// A title containing any of these words is excluded, whatever `mode`.
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Synonym {
    pub text: String,
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SeniorityLevel {
    Internship,
    Apprenticeship,
    EntryLevel,
    Associate,
    Mid,
    Senior,
    Lead,
    Director,
    Executive,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Seniority {
    pub mode: Mode,
    pub levels: Vec<SeniorityLevel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Experience {
    pub mode: Mode,
    pub max_required_years: Option<u8>,
    /// Years stated as "preferred" never exclude a posting.
    pub preferred_is_soft: bool,
}

impl Default for Experience {
    fn default() -> Self {
        Self {
            mode: Mode::Any,
            max_required_years: None,
            preferred_is_soft: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct JobTypes {
    pub mode: Mode,
    pub types: Vec<JobType>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LocationPrefs {
    pub mode: Mode,
    /// Postal code or "City, Region". Stored locally, used only for offline
    /// distance.
    pub home: Option<String>,
    /// Applies to on-site and hybrid postings.
    pub max_distance: Option<Distance>,
    /// Accepted work modes, most preferred first. Empty accepts all.
    pub work_modes: Vec<WorkMode>,
    /// Where the user can work remotely from: ISO country codes ("US") or
    /// country-region codes ("US-NY"). Empty accepts any remote posting.
    pub remote_eligible: Vec<String>,
    pub relocate: Relocate,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelocateWillingness {
    #[default]
    No,
    Listed,
    Anywhere,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Relocate {
    pub willing: RelocateWillingness,
    /// Used when `willing = "listed"`; same format as `home`.
    pub places: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PayPrefs {
    pub mode: Mode,
    pub minimum: Option<f64>,
    pub period: Option<PayPeriod>,
    /// ISO 4217; unset means USD. A posting in another currency is not
    /// compared.
    pub currency: Option<String>,
    pub keep_unlisted: bool,
    pub exclude_commission_only: bool,
}

impl Default for PayPrefs {
    fn default() -> Self {
        Self {
            mode: Mode::Any,
            minimum: None,
            period: None,
            currency: None,
            keep_unlisted: true,
            exclude_commission_only: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Freshness {
    /// Exclude postings first published (or first seen) longer ago.
    pub max_age_days: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Keywords {
    pub mode: Mode,
    /// At least one must appear in the title or description.
    pub require_any: Vec<String>,
    /// None may appear in the title or description.
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Companies {
    /// With `hard`, only these companies; with `prefer`, a score boost.
    pub allow_mode: Mode,
    pub allow: Vec<String>,
    /// Always excluded.
    pub block: Vec<String>,
    pub exclude_staffing_agencies: bool,
}

/// Modes for the checks whose values come from the verified banks
/// (§7.3 B, C, D, G) rather than from this file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Requirements {
    pub sponsorship: Mode,
    pub clearance: Mode,
    pub education: Mode,
    pub licenses: Mode,
}

/// Staging limits, enforced from M5.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Volume {
    pub nightly_cap: Option<u32>,
    pub company_cooldown_days: Option<u32>,
    pub max_active_per_company: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_an_empty_profile() {
        let p: SearchProfile = toml::from_str("").unwrap();
        assert_eq!(p, SearchProfile::default());
        assert!(p.pay.keep_unlisted);
        assert!(p.experience.preferred_is_soft);
    }

    #[test]
    fn typo_is_rejected() {
        let err = toml::from_str::<SearchProfile>("[titles]\ntargetz = [\"x\"]\n").unwrap_err();
        assert!(err.to_string().contains("targetz"), "{err}");
    }

    #[test]
    fn full_section_parses() {
        let p: SearchProfile = toml::from_str(
            r#"
            occupations = ["technology"]
            [titles]
            mode = "hard"
            targets = ["Software Engineer"]
            synonyms = [{ text = "Software Developer", verified = true }]
            exclude = ["Senior"]
            [location]
            mode = "hard"
            home = "98101"
            max_distance = { value = 25, unit = "mi" }
            work_modes = ["remote", "hybrid"]
            remote_eligible = ["US"]
            [pay]
            mode = "hard"
            minimum = 100000
            period = "year"
            currency = "USD"
            "#,
        )
        .unwrap();
        assert_eq!(p.titles.mode, Mode::Hard);
        assert_eq!(p.location.work_modes, [WorkMode::Remote, WorkMode::Hybrid]);
        assert_eq!(p.pay.period, Some(PayPeriod::Year));
    }
}
