//! One module per source. Each turns a saved or live response body into
//! normalized postings or leads; none of them makes requests.

pub mod ashby;
pub mod greenhouse;
pub mod hn;
pub mod lever;
pub mod simplify;

use gantry_core::text::{contains_phrase, normalize};
use gantry_core::{JobType, PayPeriod, WorkMode};

#[derive(Debug, thiserror::Error)]
#[error("{source_name}: unexpected response: {message}")]
pub struct ParseError {
    pub source_name: &'static str,
    pub message: String,
}

impl ParseError {
    pub(crate) fn json(source_name: &'static str, e: &serde_json::Error) -> Self {
        Self {
            source_name,
            message: e.to_string(),
        }
    }
}

/// Phrases mapped to job types, checked in order on normalized text.
/// Occupation-neutral: these are employment arrangements, not roles.
const JOB_TYPE_PHRASES: &[(&str, JobType)] = &[
    ("contract to hire", JobType::ContractToHire),
    ("full time", JobType::FullTime),
    ("fulltime", JobType::FullTime),
    ("permanent", JobType::FullTime),
    ("part time", JobType::PartTime),
    ("parttime", JobType::PartTime),
    ("internship", JobType::Internship),
    ("intern", JobType::Internship),
    ("co op", JobType::Internship),
    ("apprenticeship", JobType::Apprenticeship),
    ("apprentice", JobType::Apprenticeship),
    ("per diem", JobType::PerDiem),
    ("prn", JobType::PerDiem),
    ("seasonal", JobType::Seasonal),
    ("temporary", JobType::Temporary),
    ("fixed term", JobType::Temporary),
    ("contractor", JobType::Contract),
    ("contract", JobType::Contract),
    ("freelance", JobType::Freelance),
];

/// A job type from an ATS employment field ("Full-time", "FullTime",
/// "Contractor") or, as a fallback, from words in the title.
pub(crate) fn job_type_from_text(text: &str) -> Option<JobType> {
    let norm = normalize(text);
    JOB_TYPE_PHRASES
        .iter()
        .find(|(phrase, _)| contains_phrase(&norm, phrase))
        .map(|(_, t)| *t)
}

pub(crate) fn work_mode_from_text(text: &str) -> Option<WorkMode> {
    match normalize(text).replace(' ', "").as_str() {
        "remote" | "fullyremote" => Some(WorkMode::Remote),
        "hybrid" => Some(WorkMode::Hybrid),
        "onsite" | "inoffice" | "inperson" | "office" => Some(WorkMode::Onsite),
        _ => None,
    }
}

/// RFC 3339 timestamp → Unix seconds.
pub(crate) fn unix_seconds(text: &str) -> Option<i64> {
    text.parse::<jiff::Timestamp>()
        .ok()
        .map(jiff::Timestamp::as_second)
}

/// Lever "per-year-salary", Ashby "1 YEAR" and similar.
pub(crate) fn pay_period(text: &str) -> Option<PayPeriod> {
    let t = text.to_ascii_lowercase();
    if t.contains("year") || t.contains("annual") {
        Some(PayPeriod::Year)
    } else if t.contains("month") {
        Some(PayPeriod::Month)
    } else if t.contains("week") {
        Some(PayPeriod::Week)
    } else if t.contains("day") {
        Some(PayPeriod::Day)
    } else if t.contains("hour") {
        Some(PayPeriod::Hour)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_types_from_ats_fields_and_titles() {
        assert_eq!(job_type_from_text("Full-time"), Some(JobType::FullTime));
        assert_eq!(job_type_from_text("FullTime"), Some(JobType::FullTime));
        assert_eq!(job_type_from_text("Contractor"), Some(JobType::Contract));
        assert_eq!(job_type_from_text("Fixed-Term"), Some(JobType::Temporary));
        assert_eq!(
            job_type_from_text("Software Engineering Intern, Summer 2027"),
            Some(JobType::Internship)
        );
        assert_eq!(job_type_from_text("Internal Tools Engineer"), None);
        assert_eq!(
            job_type_from_text("Dental Hygienist (PRN)"),
            Some(JobType::PerDiem)
        );
    }

    #[test]
    fn timestamps_and_periods() {
        assert_eq!(
            unix_seconds("2026-09-09T04:35:19-04:00"),
            Some(1_788_942_919)
        );
        assert_eq!(
            unix_seconds("2026-03-12T16:38:15.322+00:00"),
            Some(1_773_333_495)
        );
        assert_eq!(pay_period("per-hour-wage"), Some(PayPeriod::Hour));
        assert_eq!(pay_period("1 YEAR"), Some(PayPeriod::Year));
        assert_eq!(pay_period("one-time"), None);
    }
}
