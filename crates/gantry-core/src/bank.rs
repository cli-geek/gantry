//! The parts of the verified banks (§7.3, §9.1) that M1's filters read.
//!
//! Each value is an [`Item`] carrying its own verification flag. Filters use
//! only verified items (invariant 3); an unverified answer is treated as
//! unknown. These structs read a subset of each file and ignore the rest,
//! because the full bank schemas arrive with onboarding in M2.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::facts::ClearanceLevel;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Item<T> {
    pub id: String,
    pub value: T,
    pub verified: bool,
    #[schemars(with = "String")]
    pub created_at: Timestamp,
    #[schemars(with = "String")]
    pub updated_at: Timestamp,
}

/// The value of an item, only if the user has verified it.
pub fn verified<T>(item: Option<&Item<T>>) -> Option<&T> {
    item.filter(|i| i.verified).map(|i| &i.value)
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EducationLevel {
    None,
    HighSchool,
    SomeCollege,
    Certificate,
    Associate,
    Bachelors,
    Masters,
    Professional,
    Doctorate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClearanceStatus {
    Active,
    /// Inactive for less than 24 months.
    Current,
    Expired,
}

/// `profile.toml`, section B.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileBank {
    pub education: Education,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Education {
    pub highest_level: Option<Item<EducationLevel>>,
}

/// `legal.toml`, sections C and D.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegalBank {
    pub authorization: Authorization,
    pub clearance: Clearance,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Authorization {
    pub require_sponsorship_now: Option<Item<bool>>,
    pub require_sponsorship_future: Option<Item<bool>>,
    pub us_citizen: Option<Item<bool>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clearance {
    pub level: Option<Item<ClearanceLevel>>,
    pub status: Option<Item<ClearanceStatus>>,
    pub willing_to_obtain: Option<Item<bool>>,
}

/// `experience.toml`, section G (licenses only in M1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExperienceBank {
    pub licenses: Vec<Item<String>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Banks {
    pub profile: ProfileBank,
    pub legal: LegalBank,
    pub experience: ExperienceBank,
}

impl Banks {
    /// `Some(true)` if either verified sponsorship answer is yes,
    /// `Some(false)` if both are verified no, `None` otherwise.
    pub fn needs_sponsorship(&self) -> Option<bool> {
        let auth = &self.legal.authorization;
        let now = verified(auth.require_sponsorship_now.as_ref()).copied();
        let future = verified(auth.require_sponsorship_future.as_ref()).copied();
        match (now, future) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        }
    }

    pub fn us_citizen(&self) -> Option<bool> {
        verified(self.legal.authorization.us_citizen.as_ref()).copied()
    }

    /// The clearance level the user holds right now (active or current),
    /// or `None` if not verified.
    pub fn held_clearance(&self) -> Option<ClearanceLevel> {
        let c = &self.legal.clearance;
        let level = *verified(c.level.as_ref())?;
        if level == ClearanceLevel::None {
            return Some(ClearanceLevel::None);
        }
        match verified(c.status.as_ref())? {
            ClearanceStatus::Active | ClearanceStatus::Current => Some(level),
            ClearanceStatus::Expired => Some(ClearanceLevel::None),
        }
    }

    pub fn willing_to_obtain_clearance(&self) -> Option<bool> {
        verified(self.legal.clearance.willing_to_obtain.as_ref()).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item<T>(value: T, verified: bool) -> Option<Item<T>> {
        Some(Item {
            id: "x".into(),
            value,
            verified,
            created_at: Timestamp::UNIX_EPOCH,
            updated_at: Timestamp::UNIX_EPOCH,
        })
    }

    #[test]
    fn unverified_answers_are_unknown() {
        let mut banks = Banks::default();
        banks.legal.authorization.require_sponsorship_now = item(true, false);
        assert_eq!(banks.needs_sponsorship(), None);
        banks.legal.authorization.require_sponsorship_now = item(false, true);
        banks.legal.authorization.require_sponsorship_future = item(false, true);
        assert_eq!(banks.needs_sponsorship(), Some(false));
        banks.legal.authorization.require_sponsorship_future = item(true, true);
        assert_eq!(banks.needs_sponsorship(), Some(true));
    }

    #[test]
    fn expired_clearance_counts_as_none() {
        let mut banks = Banks::default();
        banks.legal.clearance.level = item(ClearanceLevel::Secret, true);
        banks.legal.clearance.status = item(ClearanceStatus::Expired, true);
        assert_eq!(banks.held_clearance(), Some(ClearanceLevel::None));
        banks.legal.clearance.status = item(ClearanceStatus::Active, true);
        assert_eq!(banks.held_clearance(), Some(ClearanceLevel::Secret));
    }

    #[test]
    fn legal_toml_parses_items() {
        let bank: LegalBank = toml::from_str(
            r#"
            [authorization.us_citizen]
            id = "c-us-citizen"
            value = true
            verified = true
            created_at = "2026-09-01T00:00:00Z"
            updated_at = "2026-09-01T00:00:00Z"
            "#,
        )
        .unwrap();
        assert_eq!(
            verified(bank.authorization.us_citizen.as_ref()),
            Some(&true)
        );
    }
}
