use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::PostingKey;
use crate::facts::PostingFacts;
use crate::location::Location;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    Remote,
    Hybrid,
    Onsite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobType {
    FullTime,
    PartTime,
    Contract,
    ContractToHire,
    Temporary,
    Seasonal,
    Internship,
    Apprenticeship,
    PerDiem,
    Freelance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PayPeriod {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl PayPeriod {
    /// Multiplier to an annual figure, assuming full-time hours
    /// (40 h/week, 52 weeks, 260 working days). Used only to compare a
    /// posting's pay against the user's floor when their periods differ.
    pub const fn per_year(self) -> f64 {
        match self {
            Self::Hour => 2080.0,
            Self::Day => 260.0,
            Self::Week => 52.0,
            Self::Month => 12.0,
            Self::Year => 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pay {
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// ISO 4217 code.
    pub currency: String,
    pub period: PayPeriod,
}

impl Pay {
    /// The top of the range, annualized: the most this posting could pay.
    pub fn annual_ceiling(&self) -> Option<f64> {
        self.max.or(self.min).map(|v| v * self.period.per_year())
    }
}

/// A posting normalized from any source. Produced by discovery adapters,
/// stored by the store, read by filters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Posting {
    pub key: PostingKey,
    pub company_name: String,
    pub title: String,
    pub url: String,
    /// Location text exactly as the source gave it, one entry per listed
    /// location.
    pub locations_raw: Vec<String>,
    /// Parsed and geocoded form of `locations_raw`.
    pub locations: Vec<Location>,
    /// Workplace type from a structured ATS field, applying to every
    /// location that does not state its own.
    pub work_mode: Option<WorkMode>,
    pub job_type: Option<JobType>,
    /// Unix seconds.
    pub published_at: Option<i64>,
    /// Plain text; HTML already stripped.
    pub description: String,
    /// Facts from structured fields; text detection fills the gaps later.
    pub facts: PostingFacts,
}
