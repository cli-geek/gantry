//! Facts about a posting that filters compare against the user's profile.
//!
//! Structured ATS and feed fields fill them first. Where those are silent,
//! conservative patterns over the description fill a few more. Everything
//! else (education, licenses, seniority) waits for quarantined extraction
//! (§4.4, M3). A pattern that misses leaves the fact unknown, which never
//! excludes a posting; patterns are therefore written to avoid false hits
//! rather than to catch every phrasing.

use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Pay, PayPeriod};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FactSource {
    /// A structured field of the ATS API.
    Ats,
    /// A structured field of a list feed.
    Feed,
    /// A deterministic pattern over the description text.
    Pattern,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fact<T> {
    pub value: T,
    pub source: FactSource,
    /// The text the value was read from, shown in the list view.
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Sponsorship {
    Offered,
    NotOffered,
    /// Citizenship of the job's country is required.
    CitizenshipRequired,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ClearanceLevel {
    None,
    PublicTrust,
    Confidential,
    Secret,
    TopSecret,
    TsSci,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClearanceRequirement {
    pub level: ClearanceLevel,
    /// The posting requires a clearance already held, as opposed to the
    /// ability to obtain one.
    pub active_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct YearsRequirement {
    pub years: u8,
    /// Stated as preferred or a plus rather than required.
    pub preferred: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PostingFacts {
    pub sponsorship: Option<Fact<Sponsorship>>,
    pub clearance: Option<Fact<ClearanceRequirement>>,
    pub min_years: Option<Fact<YearsRequirement>>,
    pub pay: Option<Fact<Pay>>,
    pub commission_only: Option<Fact<bool>>,
}

impl PostingFacts {
    /// Fills facts still unknown from patterns over `text`. Facts already
    /// set from structured fields are kept.
    pub fn fill_from_text(&mut self, text: &str) {
        if self.sponsorship.is_none() {
            self.sponsorship = detect_sponsorship(text);
        }
        if self.clearance.is_none() {
            self.clearance = detect_clearance(text);
        }
        if self.min_years.is_none() {
            self.min_years = detect_min_years(text);
        }
        if self.pay.is_none() {
            self.pay = detect_pay(text);
        }
        if self.commission_only.is_none() {
            self.commission_only = detect_commission_only(text);
        }
    }
}

fn pattern(source: &str) -> Regex {
    // Patterns are compile-time constants covered by tests; a bad one is a
    // programming error caught on first use in any test run.
    #[allow(clippy::expect_used)]
    Regex::new(source).expect("valid fact pattern")
}

fn sentence_around(text: &str, start: usize, end: usize) -> &str {
    let s = text[..start]
        .rfind(['.', '\n', '!', '?', ';'])
        .map_or(0, |i| i + 1);
    let e = text[end..]
        .find(['.', '\n', '!', '?', ';'])
        .map_or(text.len(), |i| end + i);
    text[s..e].trim()
}

/// The part of `text`'s sentence that comes before `start`.
fn lead_in(text: &str, start: usize) -> &str {
    let s = text[..start]
        .rfind(['.', '\n', '!', '?', ';'])
        .map_or(0, |i| i + 1);
    &text[s..start]
}

fn fact<T>(value: T, evidence: &str) -> Fact<T> {
    Fact {
        value,
        source: FactSource::Pattern,
        evidence: evidence.chars().take(240).collect(),
    }
}

static NO_SPONSOR: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"(?i)\b(?:",
        r"(?:unable|not able|cannot|can ?not|can't|will not|won't|do not|does not|don't|doesn't|are not able|is not able)",
        r"\s+(?:to\s+)?(?:currently\s+)?(?:(?:provide|offer|support|consider)\s+(?:\w+\s+){0,3})?sponsor",
        r"|without\s+(?:the\s+need\s+for\s+|requiring\s+|needing\s+)?(?:current\s+or\s+future\s+|future\s+|any\s+)?(?:\w+\s+){0,2}?sponsorship",
        r"|sponsorship\s+(?:is\s+)?(?:not\s+(?:available|offered|provided|possible)|unavailable)",
        r"|no\s+(?:visa\s+|immigration\s+)?sponsorship",
        r")"
    ))
});

static CITIZENSHIP: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"(?i)\b(?:",
        r"(?:u\.?s\.?|united states)\s+citizenship\s+(?:is\s+)?required",
        r"|must\s+(?:be|hold)\s+(?:a\s+)?(?:u\.?s\.?|united states)\s+citizen",
        r"|requires?\s+(?:u\.?s\.?|united states)\s+citizenship",
        r"|only\s+(?:u\.?s\.?|united states)\s+citizens",
        r")"
    ))
});

/// "does not require U.S. citizenship": a negation shortly before.
static NEGATED: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)(?:\b(?:not|no|never)|n't)\s+(?:\w+\s+){0,2}$"));

/// "U.S. citizen or permanent resident": citizenship is one of several
/// statuses accepted, none of which needs sponsorship. A comma alone
/// ("citizen, per ITAR") does not start such a list.
static CITIZEN_OR: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)^s?\s*(?:/|,?\s*\bor\b|\band/or\b|,\s*(?:permanent|green|lawful|national|asylee|refugee))",
    )
});

/// "with or without sponsorship needs" welcomes both.
static WITH_OR: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?i)\bwith\s+(?:or|and)\s+$"));

pub fn detect_sponsorship(text: &str) -> Option<Fact<Sponsorship>> {
    let (either_or, strict): (Vec<_>, Vec<_>) = CITIZENSHIP
        .find_iter(text)
        .filter(|m| !NEGATED.is_match(lead_in(text, m.start())))
        .partition(|m| CITIZEN_OR.is_match(&text[m.end()..]));
    if let Some(m) = strict.first() {
        return Some(fact(
            Sponsorship::CitizenshipRequired,
            sentence_around(text, m.start(), m.end()),
        ));
    }
    if let Some(m) = either_or.first() {
        return Some(fact(
            Sponsorship::NotOffered,
            sentence_around(text, m.start(), m.end()),
        ));
    }
    NO_SPONSOR
        .find_iter(text)
        .find(|m| !WITH_OR.is_match(lead_in(text, m.start())))
        .map(|m| {
            fact(
                Sponsorship::NotOffered,
                sentence_around(text, m.start(), m.end()),
            )
        })
}

static CLEARANCE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"(?i)\b(?:(active|current)\s+)?",
        r"(ts\s*/\s*sci|top\s+secret(?:\s*/\s*sci)?|secret|public\s+trust|confidential)",
        r"(?:\s+(?:security\s+)?clearance\b|\s+(?:with|w/)\s+(?:full[- ]scope\s+|ci\s+)?poly)",
        r"|\b(ts\s*/\s*sci)\b"
    ))
});

static CLEARANCE_OBTAINABLE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"(?i)\b(?:(?:ability|able|eligible|eligibility|willing(?:ness)?)\s+to\s+(?:obtain|acquire|get|attain)",
        r"|obtainable)\b",
    ))
});

static CLEARANCE_NOT_REQUIRED: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)\b(?:not\s+required|no\s+(?:security\s+)?clearance|is\s+a\s+plus|preferred|nice\s+to\s+have)\b",
    )
});

/// Read just before the clearance, so a negation or "eligible for"
/// elsewhere in the sentence ("…which does not require relocation", "…and
/// be eligible for a polygraph") does not apply to it.
static CLEARANCE_NOT_NEEDED_BEFORE: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)(?:\bnot|n't)\s+(?:require[sd]?|need)\s+(?:\w+\s+){0,3}$"));
static CLEARANCE_ELIGIBLE_BEFORE: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)\b(?:eligible|eligibility)\s+for\s+(?:\w+\s+){0,3}$"));

pub fn detect_clearance(text: &str) -> Option<Fact<ClearanceRequirement>> {
    CLEARANCE.captures_iter(text).find_map(|caps| {
        let whole = caps.get(0)?;
        let sentence = sentence_around(text, whole.start(), whole.end());
        let before = lead_in(text, whole.start());
        if CLEARANCE_NOT_REQUIRED.is_match(sentence) || CLEARANCE_NOT_NEEDED_BEFORE.is_match(before)
        {
            return None;
        }
        let level_text = caps.get(2).or_else(|| caps.get(3))?.as_str().to_lowercase();
        let squashed: String = level_text.split_whitespace().collect();
        let level = if squashed.contains("sci") {
            ClearanceLevel::TsSci
        } else if squashed.starts_with("topsecret") {
            ClearanceLevel::TopSecret
        } else if squashed == "secret" {
            ClearanceLevel::Secret
        } else if squashed == "publictrust" {
            ClearanceLevel::PublicTrust
        } else {
            ClearanceLevel::Confidential
        };
        let obtainable =
            CLEARANCE_OBTAINABLE.is_match(sentence) || CLEARANCE_ELIGIBLE_BEFORE.is_match(before);
        let active_required = caps.get(1).is_some() || !obtainable;
        Some(fact(
            ClearanceRequirement {
                level,
                active_required,
            },
            sentence,
        ))
    })
}

static YEARS: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"(?i)\b(\d{1,2})\s*(?:\+|plus)?\s*(?:(?:-|–|to)\s*\d{1,2}\s*)?\+?\s*(?:years?|yrs?)['’]?",
        r"(?:\s+of)?[^.\n;]{0,60}?\bexperience\b",
    ))
});

/// "at least 18 years old", "21 years of age".
static AGE: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)\b(?:years?|yrs?)\s+(?:old|of\s+age)\b"));

/// The company's history, not a requirement: "With 25 years of
/// experience serving customers, we…", "Our 30 years…", "We bring 20+
/// years…", "For over 30 years…". A sentence opening with "With" is read
/// as history; "candidates with 3+ years" is not.
static COMPANY_HISTORY: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"(?i)(?:\bour|\bwe(?:'ve|\s+have|\s+bring|\s+boast)|^\s*with",
        r"|\bfor\s+(?:over|more\s+than|nearly|almost))",
        r"\s+(?:over\s+|more\s+than\s+|nearly\s+|almost\s+)?$",
    ))
});

/// "50 years of combined experience" is a team's total.
static COMBINED: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)\b(?:combined|collective|cumulative)\b"));

static PREFERRED: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?i)\b(?:preferred|nice to have|a plus|bonus|ideally|desired|desirable)\b")
});

/// The smallest stated years-of-experience figure. Taking the minimum
/// errs toward keeping a posting; the review screen shows the evidence.
pub fn detect_min_years(text: &str) -> Option<Fact<YearsRequirement>> {
    YEARS
        .captures_iter(text)
        .filter_map(|caps| {
            let whole = caps.get(0)?;
            if AGE.is_match(whole.as_str())
                || COMBINED.is_match(whole.as_str())
                || COMPANY_HISTORY.is_match(lead_in(text, whole.start()))
            {
                return None;
            }
            let years: u8 = caps.get(1)?.as_str().parse().ok()?;
            let sentence = sentence_around(text, whole.start(), whole.end());
            Some(fact(
                YearsRequirement {
                    years,
                    preferred: PREFERRED.is_match(sentence),
                },
                sentence,
            ))
        })
        .min_by_key(|f| (f.value.preferred, f.value.years))
}

const AMOUNT: &str = r"(\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?)\s*([kK])?";

static PAY: LazyLock<Regex> = LazyLock::new(|| {
    pattern(&format!(
        r"(?i)(CA\$|C\$|A\$|AU\$|US\$|\$|£|€)\s?{AMOUNT}\s*(?:USD|CAD|AUD|GBP|EUR)?\s*(?:-|–|—|to)\s*(?:CA\$|C\$|A\$|AU\$|US\$|\$|£|€)?\s?{AMOUNT}\s*(USD|CAD|AUD|GBP|EUR)?(?:\s*(?:/|per|an|a)\s*(hour|hr|year|yr|annum|month|mo|week|wk|day))?",
    ))
});

/// A range named a stipend or budget in the words around it is not the
/// salary, unless the words just before it say salary. Bonus and
/// relocation are left out: "$150k - $180k + bonus" is a salary.
static NOT_SALARY: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)\b(?:stipend|budget|allowance|reimburse\w*|per\s+diem)\b"));

/// The last few words before `start` and the first few after `end`.
fn words_around(text: &str, start: usize, end: usize) -> (String, String) {
    const WORDS: usize = 6;
    let before: Vec<&str> = lead_in(text, start).split_whitespace().collect();
    let before = before[before.len().saturating_sub(WORDS)..].join(" ");
    let after = text[end..]
        .split_whitespace()
        .take(WORDS)
        .collect::<Vec<_>>()
        .join(" ");
    (before, after)
}
static SALARY: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)\b(?:salary|pay|compensation|wages?|base|rate|earn\w*|ote)\b"));

fn amount(digits: &str, thousands: bool) -> Option<f64> {
    // "50.000" is fifty thousand where a dot groups thousands; a decimal
    // part has one or two digits.
    let plain = if digits.contains('.') && digits.split('.').skip(1).all(|g| g.len() == 3) {
        digits.replace(['.', ','], "")
    } else {
        digits.replace(',', "")
    };
    let v: f64 = plain.parse().ok()?;
    Some(if thousands { v * 1000.0 } else { v })
}

/// A pay range written as "$120,000 - $150,000", "$45 to $55 per hour",
/// "£40k–£50k". A range without a stated period is taken as hourly below
/// 500 and annual above 10,000; anything in between is left unknown.
pub fn detect_pay(text: &str) -> Option<Fact<Pay>> {
    PAY.captures_iter(text).find_map(|caps| {
        let whole = caps.get(0)?;
        let rest = text[whole.end()..].trim_start().to_lowercase();
        if ["million", "billion", "m ", "bn"]
            .iter()
            .any(|w| rest.starts_with(w))
        {
            return None;
        }
        let (before, after) = words_around(text, whole.start(), whole.end());
        if (NOT_SALARY.is_match(&before) || NOT_SALARY.is_match(&after))
            && !SALARY.is_match(&before)
        {
            return None;
        }
        let symbol = caps.get(1)?.as_str().to_uppercase();
        let min = amount(caps.get(2)?.as_str(), caps.get(3).is_some())?;
        let max = amount(caps.get(4)?.as_str(), caps.get(5).is_some())?;
        if max < min {
            return None;
        }
        let currency = match caps.get(6).map(|m| m.as_str().to_uppercase()) {
            Some(code) => code,
            None => match symbol.as_str() {
                "CA$" | "C$" => "CAD".to_owned(),
                "A$" | "AU$" => "AUD".to_owned(),
                "£" => "GBP".to_owned(),
                "€" => "EUR".to_owned(),
                _ => "USD".to_owned(),
            },
        };
        let period = match caps.get(7).map(|m| m.as_str().to_lowercase()).as_deref() {
            Some("hour" | "hr") => PayPeriod::Hour,
            Some("day") => PayPeriod::Day,
            Some("week" | "wk") => PayPeriod::Week,
            Some("month" | "mo") => PayPeriod::Month,
            Some(_) => PayPeriod::Year,
            None if max < 500.0 => PayPeriod::Hour,
            None if min >= 10_000.0 => PayPeriod::Year,
            None => return None,
        };
        Some(fact(
            Pay {
                min: Some(min),
                max: Some(max),
                currency,
                period,
            },
            whole.as_str(),
        ))
    })
}

static COMMISSION_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?i)\b(?:commission[- ]only|100\s?%\s+commission|straight\s+commission)\b")
});

pub fn detect_commission_only(text: &str) -> Option<Fact<bool>> {
    COMMISSION_ONLY
        .find(text)
        .map(|m| fact(true, sentence_around(text, m.start(), m.end())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sponsorship_negative_phrasings() {
        for text in [
            "We are unable to sponsor visas for this role.",
            "This company does not offer visa sponsorship.",
            "Candidates must be authorized to work in the US without sponsorship.",
            "Must be able to work without the need for current or future visa sponsorship.",
            "Sponsorship is not available for this position.",
            "No visa sponsorship.",
            "We cannot sponsor H-1B.",
        ] {
            let f = detect_sponsorship(text).unwrap_or_else(|| panic!("missed: {text}"));
            assert_eq!(f.value, Sponsorship::NotOffered, "{text}");
        }
    }

    #[test]
    fn sponsorship_absent_or_citizenship() {
        assert!(detect_sponsorship("We offer visa sponsorship and relocation.").is_none());
        assert!(detect_sponsorship("Great benefits, flexible hours.").is_none());
        let f = detect_sponsorship("U.S. citizenship is required due to contract terms.").unwrap();
        assert_eq!(f.value, Sponsorship::CitizenshipRequired);
    }

    #[test]
    fn clearance_levels_and_obtainability() {
        let f = detect_clearance("Must hold an active TS/SCI clearance with polygraph.").unwrap();
        assert_eq!(f.value.level, ClearanceLevel::TsSci);
        assert!(f.value.active_required);

        let f = detect_clearance("Ability to obtain a Secret clearance.").unwrap();
        assert_eq!(f.value.level, ClearanceLevel::Secret);
        assert!(!f.value.active_required);

        assert!(detect_clearance("A Secret clearance is a plus.").is_none());
        assert!(detect_clearance("Keep patient records confidential.").is_none());
        assert!(detect_clearance("Our secret sauce is people.").is_none());
    }

    #[test]
    fn years_of_experience() {
        let f =
            detect_min_years("Requirements: 5+ years of professional experience in Go.").unwrap();
        assert_eq!(f.value.years, 5);
        assert!(!f.value.preferred);

        let f = detect_min_years(
            "3-5 years of experience with SQL. 1 year of clinical experience preferred.",
        )
        .unwrap();
        assert_eq!(
            f.value,
            YearsRequirement {
                years: 3,
                preferred: false
            }
        );

        let f = detect_min_years("2 years' experience in a dental office is a plus.").unwrap();
        assert!(f.value.preferred);

        assert!(detect_min_years("Founded 10 years ago, we build tools.").is_none());
    }

    #[test]
    fn pay_ranges() {
        let f = detect_pay("The base salary range is $130,000 - $160,000 USD per year.").unwrap();
        assert_eq!(f.value.min, Some(130_000.0));
        assert_eq!(f.value.max, Some(160_000.0));
        assert_eq!(f.value.currency, "USD");
        assert_eq!(f.value.period, PayPeriod::Year);

        let f = detect_pay("Pay: $45 to $55 per hour").unwrap();
        assert_eq!(f.value.period, PayPeriod::Hour);

        let f = detect_pay("£40k–£50k").unwrap();
        assert_eq!(f.value.currency, "GBP");
        assert_eq!(f.value.max, Some(50_000.0));
        assert_eq!(f.value.period, PayPeriod::Year);

        let f = detect_pay("Compensation: $120K - $150K").unwrap();
        assert_eq!(f.value.min, Some(120_000.0));

        assert!(detect_pay("We raised $40 - $50 million in funding").is_none());
        assert!(detect_pay("We have 500 - 900 employees").is_none());
        assert!(detect_pay("$2,000 - $3,000 signing bonus").is_none());
    }

    #[test]
    fn review_false_hits_stay_unknown() {
        for text in [
            "This role does not require U.S. citizenship.",
            "We welcome applicants with or without sponsorship needs.",
        ] {
            assert_eq!(detect_sponsorship(text), None, "{text}");
        }
        assert_eq!(
            detect_sponsorship("We do not sponsor. Must be a U.S. citizen.")
                .unwrap()
                .value,
            Sponsorship::CitizenshipRequired
        );

        assert!(detect_clearance("This role does not require a Secret clearance.").is_none());
        assert!(detect_clearance("This role doesn't require a TS/SCI clearance.").is_none());
        for text in [
            "Must be eligible for a Secret clearance.",
            "Must be able to acquire a Top Secret clearance.",
        ] {
            assert!(
                !detect_clearance(text).unwrap().value.active_required,
                "{text}"
            );
        }

        for text in [
            "With over 25 years of experience serving customers, we lead the market.",
            "Our 30 years of experience in logistics set us apart.",
            "Must be at least 18 years old. Customer service experience a plus.",
            "Must be 21 years of age; bartending experience required.",
        ] {
            assert_eq!(detect_min_years(text), None, "{text}");
        }
        assert_eq!(
            detect_min_years("Must be 18 years old. 3+ years of experience in sales.")
                .unwrap()
                .value
                .years,
            3
        );

        assert!(detect_pay("Enjoy a $500 - $1,000 annual learning stipend.").is_none());
        assert!(detect_pay("Home office budget: $300 to $600.").is_none());
        let f = detect_pay(
            "We offer a $500 - $1,000 learning budget. Salary: $120,000 - $140,000 per year.",
        )
        .unwrap();
        assert_eq!(f.value.min, Some(120_000.0));
        let f = detect_pay("Gehalt: €50.000 - €60.000 pro Jahr").unwrap();
        assert_eq!(
            (f.value.min, f.value.max, f.value.currency.as_str()),
            (Some(50_000.0), Some(60_000.0), "EUR")
        );
        assert_eq!(
            detect_pay("$45.50 - $55.25 per hour").unwrap().value.max,
            Some(55.25)
        );
    }

    #[test]
    fn second_review_cases() {
        for text in [
            "With 25 years of experience serving customers, we lead the market.",
            "Our team has 50 years of combined experience.",
            "We bring 20+ years of experience in logistics.",
            "For over 30 years of experience, we've served the region.",
        ] {
            assert_eq!(detect_min_years(text), None, "{text}");
        }
        for (text, years) in [
            ("Candidates with 3+ years of experience in Go.", 3),
            ("We require 4+ years of experience with Kubernetes.", 4),
            ("Our ideal candidate has 2+ years of experience.", 2),
        ] {
            assert_eq!(
                detect_min_years(text).map(|f| f.value.years),
                Some(years),
                "{text}"
            );
        }

        assert!(
            detect_pay(
                "We provide a $500 - $1,000 per month home office stipend on top of base salary."
            )
            .is_none()
        );
        for text in [
            "The range for this role is $150,000 - $180,000 + bonus + equity.",
            "$140,000 - $170,000 plus relocation assistance.",
            "Base salary $100,000 - $120,000 plus a learning stipend.",
        ] {
            assert!(
                detect_pay(text).is_some_and(|f| f.value.min >= Some(100_000.0)),
                "{text}"
            );
        }

        assert_eq!(
            detect_sponsorship("We consider candidates with and without sponsorship needs."),
            None
        );
        for text in [
            "U.S. citizenship is required, as this role supports federal contracts.",
            "Must be a U.S. citizen, per ITAR regulations.",
        ] {
            assert_eq!(
                detect_sponsorship(text).map(|f| f.value),
                Some(Sponsorship::CitizenshipRequired),
                "{text}"
            );
        }
        for text in [
            "Must be a U.S. citizen or permanent resident.",
            "Applicants must be U.S. citizens, permanent residents, or asylees.",
        ] {
            assert_eq!(
                detect_sponsorship(text).map(|f| f.value),
                Some(Sponsorship::NotOffered),
                "{text}"
            );
        }

        let f = detect_clearance(
            "Active Secret clearance required for this role, which does not require relocation.",
        )
        .unwrap();
        assert!(f.value.active_required);
        let f = detect_clearance(
            "Must currently hold a Secret clearance and be eligible for a polygraph.",
        )
        .unwrap();
        assert!(f.value.active_required);
    }

    #[test]
    fn structured_facts_are_not_overwritten() {
        let mut facts = PostingFacts {
            sponsorship: Some(Fact {
                value: Sponsorship::Offered,
                source: FactSource::Feed,
                evidence: "Offers Sponsorship".into(),
            }),
            ..PostingFacts::default()
        };
        facts.fill_from_text("We are unable to sponsor visas. 2+ years of experience.");
        assert_eq!(facts.sponsorship.unwrap().value, Sponsorship::Offered);
        assert_eq!(facts.min_years.unwrap().value.years, 2);
    }
}
