//! Deterministic filters (§4.3), run before any model call.
//!
//! Each configured filter yields a [`Check`] with a reason the list view
//! shows. A posting is excluded only by a failing check whose section is
//! `hard`. Missing information gives `Unknown`, which never excludes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::bank::Banks;
use crate::facts::Sponsorship;
use crate::geo::{DistanceUnit, Place, haversine_km};
use crate::location::{Area, Location};
use crate::search::{Mode, RelocateWillingness, SearchProfile};
use crate::text::{contains_phrase, normalize};
use crate::{JobType, Pay, Posting, WorkMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilterName {
    Title,
    Seniority,
    Experience,
    JobType,
    Location,
    Pay,
    Freshness,
    Keywords,
    Company,
    Sponsorship,
    Clearance,
    Education,
    Licenses,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Fail,
    Unknown,
    Pass,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Check {
    pub filter: FilterName,
    /// Whether a `fail` here excludes the posting.
    pub hard: bool,
    pub verdict: Verdict,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FilterOutcome {
    pub passed: bool,
    pub checks: Vec<Check>,
}

/// Everything about the user a filter may read.
#[derive(Debug, Clone, Copy)]
pub struct FilterContext<'a> {
    pub search: &'a SearchProfile,
    pub banks: &'a Banks,
    /// `search.location.home`, geocoded.
    pub home: Option<&'a Place>,
    /// `search.location.relocate.places`, geocoded.
    pub relocate_places: &'a [Place],
    /// Unix seconds.
    pub now: i64,
}

/// A stored posting plus what the store knows about it.
#[derive(Debug, Clone, Copy)]
pub struct Subject<'a> {
    pub posting: &'a Posting,
    /// Unix seconds; stands in for a missing publish date.
    pub first_seen: i64,
    /// From the company list; `None` when not known.
    pub staffing_agency: Option<bool>,
}

pub fn evaluate(subject: &Subject<'_>, ctx: &FilterContext<'_>) -> FilterOutcome {
    let checks: Vec<Check> = [
        title(subject, ctx),
        seniority(subject, ctx),
        experience(subject, ctx),
        job_type(subject, ctx),
        location(subject, ctx),
        pay(subject, ctx),
        commission(subject, ctx),
        freshness(subject, ctx),
        keywords(subject, ctx),
        company(subject, ctx),
        sponsorship(subject, ctx),
        clearance(subject, ctx),
        not_checked_yet(FilterName::Education, ctx.search.requirements.education),
        not_checked_yet(FilterName::Licenses, ctx.search.requirements.licenses),
    ]
    .into_iter()
    .flatten()
    .collect();
    FilterOutcome {
        passed: !checks.iter().any(|c| c.hard && c.verdict == Verdict::Fail),
        checks,
    }
}

fn check(filter: FilterName, mode: Mode, verdict: Verdict, detail: impl Into<String>) -> Check {
    Check {
        filter,
        hard: mode == Mode::Hard,
        verdict,
        detail: detail.into(),
    }
}

fn title(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let t = &ctx.search.titles;
    let norm = normalize(&s.posting.title);
    if let Some(term) = t.exclude.iter().find(|e| contains_phrase(&norm, e)) {
        return Some(check(
            FilterName::Title,
            Mode::Hard,
            Verdict::Fail,
            format!("title contains excluded term \"{term}\""),
        ));
    }
    let candidates: Vec<&str> = t
        .targets
        .iter()
        .map(String::as_str)
        .chain(
            t.synonyms
                .iter()
                .filter(|s| s.verified)
                .map(|s| s.text.as_str()),
        )
        .collect();
    if t.mode == Mode::Any || candidates.is_empty() {
        return None;
    }
    Some(
        match candidates.iter().find(|c| contains_phrase(&norm, c)) {
            Some(hit) => check(
                FilterName::Title,
                t.mode,
                Verdict::Pass,
                format!("matches \"{hit}\""),
            ),
            None => check(
                FilterName::Title,
                t.mode,
                Verdict::Fail,
                "matches none of your target titles",
            ),
        },
    )
}

fn seniority(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    use crate::search::SeniorityLevel;
    let pref = &ctx.search.seniority;
    if pref.mode == Mode::Any || pref.levels.is_empty() {
        return None;
    }
    let level = match s.posting.job_type {
        Some(JobType::Internship) => Some(SeniorityLevel::Internship),
        Some(JobType::Apprenticeship) => Some(SeniorityLevel::Apprenticeship),
        _ => None,
    };
    Some(match level {
        Some(l) if pref.levels.contains(&l) => check(
            FilterName::Seniority,
            pref.mode,
            Verdict::Pass,
            format!("{l:?}"),
        ),
        Some(l) => check(
            FilterName::Seniority,
            pref.mode,
            Verdict::Fail,
            format!("{l:?} is not one of your levels"),
        ),
        None => check(
            FilterName::Seniority,
            pref.mode,
            Verdict::Unknown,
            "not checked yet: needs posting extraction",
        ),
    })
}

fn experience(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let pref = &ctx.search.experience;
    let max = pref.max_required_years?;
    if pref.mode == Mode::Any {
        return None;
    }
    let Some(req) = &s.posting.facts.min_years else {
        return Some(check(
            FilterName::Experience,
            pref.mode,
            Verdict::Pass,
            "no years-of-experience requirement found",
        ));
    };
    let years = req.value.years;
    Some(if years <= max {
        check(
            FilterName::Experience,
            pref.mode,
            Verdict::Pass,
            format!("asks for {years}+ years"),
        )
    } else if req.value.preferred && pref.preferred_is_soft {
        check(
            FilterName::Experience,
            pref.mode,
            Verdict::Pass,
            format!("{years}+ years preferred, not required"),
        )
    } else {
        check(
            FilterName::Experience,
            pref.mode,
            Verdict::Fail,
            format!(
                "asks for {years}+ years; your limit is {max}: \"{}\"",
                req.evidence
            ),
        )
    })
}

fn job_type(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let pref = &ctx.search.job_type;
    if pref.mode == Mode::Any || pref.types.is_empty() {
        return None;
    }
    Some(match s.posting.job_type {
        None => check(
            FilterName::JobType,
            pref.mode,
            Verdict::Unknown,
            "job type not stated",
        ),
        Some(t) if pref.types.contains(&t) => check(
            FilterName::JobType,
            pref.mode,
            Verdict::Pass,
            format!("{t:?}"),
        ),
        Some(t) => check(
            FilterName::JobType,
            pref.mode,
            Verdict::Fail,
            format!("{t:?} is not one of your job types"),
        ),
    })
}

fn location(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let pref = &ctx.search.location;
    if pref.mode == Mode::Any {
        return None;
    }
    let posting = s.posting;
    let mut results: Vec<(Verdict, String)> = Vec::new();
    if posting.locations.is_empty() {
        match posting.work_mode {
            Some(mode) => results.push(judge_location(None, mode, ctx)),
            None => results.push((Verdict::Unknown, "no location listed".into())),
        }
    }
    for loc in &posting.locations {
        let modes: Vec<WorkMode> = if !loc.modes.is_empty() {
            loc.modes.clone()
        } else if let Some(m) = posting.work_mode {
            vec![m]
        } else {
            // Work mode not stated: decide only if every mode gives the
            // same verdict, so a remote or hybrid job listed under a city
            // is not judged as on-site.
            let judged = [WorkMode::Onsite, WorkMode::Hybrid, WorkMode::Remote]
                .map(|m| judge_location(Some(loc), m, ctx));
            if judged.iter().all(|(v, _)| *v == judged[0].0) {
                let [onsite, ..] = judged;
                results.push(onsite);
            } else {
                results.push((
                    Verdict::Unknown,
                    format!("\"{}\": work mode not stated", loc.raw),
                ));
            }
            continue;
        };
        for mode in modes {
            results.push(judge_location(Some(loc), mode, ctx));
        }
    }
    let best = results.iter().map(|(v, _)| *v).max()?;
    let detail = results
        .iter()
        .filter(|(v, _)| *v == best)
        .map(|(_, d)| d.as_str())
        .take(3)
        .collect::<Vec<_>>()
        .join("; ");
    Some(check(FilterName::Location, pref.mode, best, detail))
}

fn mode_name(mode: WorkMode) -> &'static str {
    match mode {
        WorkMode::Remote => "remote",
        WorkMode::Hybrid => "hybrid",
        WorkMode::Onsite => "on-site",
    }
}

fn judge_location(
    loc: Option<&Location>,
    mode: WorkMode,
    ctx: &FilterContext<'_>,
) -> (Verdict, String) {
    let pref = &ctx.search.location;
    let label = loc.map_or_else(
        || mode_name(mode).to_owned(),
        |l| format!("{} ({})", l.raw, mode_name(mode)),
    );
    if !pref.work_modes.is_empty() && !pref.work_modes.contains(&mode) {
        return (
            Verdict::Fail,
            format!("{label}: {} not accepted", mode_name(mode)),
        );
    }
    if mode == WorkMode::Remote {
        return judge_remote(loc, &label, &pref.remote_eligible);
    }
    let Some(max) = pref.max_distance else {
        return (Verdict::Pass, label);
    };
    let Some(place) = loc.and_then(|l| l.place.as_ref()) else {
        let what = loc
            .and_then(|l| l.unresolved.clone())
            .unwrap_or_else(|| "location".into());
        return (
            Verdict::Unknown,
            format!("{label}: could not place \"{what}\""),
        );
    };
    if pref.relocate.willing == RelocateWillingness::Anywhere {
        return (
            Verdict::Pass,
            format!("{label}: willing to relocate anywhere"),
        );
    }
    let unit = match max.unit {
        DistanceUnit::Mi => "mi",
        DistanceUnit::Km => "km",
    };
    let to_unit = |km: f64| match max.unit {
        DistanceUnit::Mi => km / 1.609_344,
        DistanceUnit::Km => km,
    };
    if pref.relocate.willing == RelocateWillingness::Listed
        && let Some(target) = ctx
            .relocate_places
            .iter()
            .find(|t| haversine_km(t, place) <= max.km())
    {
        return (
            Verdict::Pass,
            format!("{label}: near relocation target {}", target.name),
        );
    }
    let Some(home) = ctx.home else {
        return (
            Verdict::Unknown,
            format!("{label}: home location not set or not found"),
        );
    };
    let d = haversine_km(home, place);
    if d <= max.km() {
        (
            Verdict::Pass,
            format!("{label}: {:.0} {unit} from home", to_unit(d)),
        )
    } else {
        (
            Verdict::Fail,
            format!(
                "{label}: {:.0} {unit} from home, limit {}",
                to_unit(d),
                max.value
            ),
        )
    }
}

fn judge_remote(loc: Option<&Location>, label: &str, eligible: &[String]) -> (Verdict, String) {
    if eligible.is_empty() {
        return (Verdict::Pass, label.to_owned());
    }
    let area = loc.and_then(Location::effective_area);
    let entries: Vec<(&str, Option<&str>)> = eligible
        .iter()
        .map(|e| match e.split_once('-') {
            Some((c, r)) => (c, Some(r)),
            None => (e.as_str(), None),
        })
        .collect();
    match area {
        None => (Verdict::Unknown, format!("{label}: region not stated")),
        Some(Area::Anywhere) => (Verdict::Pass, format!("{label}: anywhere")),
        Some(Area::Continent { code }) => (
            Verdict::Unknown,
            format!("{label}: open within continent {code}"),
        ),
        Some(Area::Country { country }) => {
            if entries
                .iter()
                .any(|(c, _)| c.eq_ignore_ascii_case(&country))
            {
                (Verdict::Pass, format!("{label}: open in {country}"))
            } else {
                (Verdict::Fail, format!("{label}: open only in {country}"))
            }
        }
        Some(Area::Admin1 { country, admin1 }) => {
            let ok = entries.iter().any(|(c, r)| {
                c.eq_ignore_ascii_case(&country)
                    && r.is_none_or(|r| r.eq_ignore_ascii_case(&admin1))
            });
            if ok {
                (
                    Verdict::Pass,
                    format!("{label}: open in {country}-{admin1}"),
                )
            } else {
                (
                    Verdict::Fail,
                    format!("{label}: open only in {country}-{admin1}"),
                )
            }
        }
    }
}

fn format_pay(pay: &Pay) -> String {
    let n = |v: f64| {
        if v >= 1000.0 {
            format!("{:.0}k", v / 1000.0)
        } else {
            format!("{v}")
        }
    };
    let range = match (pay.min, pay.max) {
        (Some(a), Some(b)) => format!("{}-{}", n(a), n(b)),
        (Some(a), None) | (None, Some(a)) => n(a),
        (None, None) => "?".into(),
    };
    format!("{range} {}/{:?}", pay.currency, pay.period).to_lowercase()
}

fn pay(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let pref = &ctx.search.pay;
    let minimum = pref.minimum?;
    if pref.mode == Mode::Any {
        return None;
    }
    let Some(fact) = &s.posting.facts.pay else {
        return Some(if pref.keep_unlisted {
            check(
                FilterName::Pay,
                pref.mode,
                Verdict::Pass,
                "no pay listed; kept",
            )
        } else {
            check(FilterName::Pay, pref.mode, Verdict::Fail, "no pay listed")
        });
    };
    let pay = &fact.value;
    let currency = pref.currency.as_deref().unwrap_or("USD");
    if !currency.eq_ignore_ascii_case(&pay.currency) {
        return Some(check(
            FilterName::Pay,
            pref.mode,
            Verdict::Unknown,
            format!("{} is in another currency", format_pay(pay)),
        ));
    }
    let floor = minimum * pref.period.map_or(1.0, crate::PayPeriod::per_year);
    let ceiling = pay.annual_ceiling()?;
    Some(if ceiling >= floor {
        check(FilterName::Pay, pref.mode, Verdict::Pass, format_pay(pay))
    } else {
        check(
            FilterName::Pay,
            pref.mode,
            Verdict::Fail,
            format!("{} is below your minimum", format_pay(pay)),
        )
    })
}

fn commission(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    if !ctx.search.pay.exclude_commission_only {
        return None;
    }
    let fact = s
        .posting
        .facts
        .commission_only
        .as_ref()
        .filter(|f| f.value)?;
    Some(check(
        FilterName::Pay,
        Mode::Hard,
        Verdict::Fail,
        format!("commission-only: \"{}\"", fact.evidence),
    ))
}

fn freshness(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let max = ctx.search.freshness.max_age_days?;
    let since = s.posting.published_at.unwrap_or(s.first_seen);
    let days = (ctx.now - since).max(0) / 86_400;
    Some(if days <= i64::from(max) {
        check(
            FilterName::Freshness,
            Mode::Hard,
            Verdict::Pass,
            format!("{days} days old"),
        )
    } else {
        check(
            FilterName::Freshness,
            Mode::Hard,
            Verdict::Fail,
            format!("{days} days old; your limit is {max}"),
        )
    })
}

fn keywords(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let pref = &ctx.search.keywords;
    if pref.mode == Mode::Any || (pref.require_any.is_empty() && pref.exclude.is_empty()) {
        return None;
    }
    let text = normalize(&format!("{} {}", s.posting.title, s.posting.description));
    if let Some(bad) = pref.exclude.iter().find(|k| contains_phrase(&text, k)) {
        return Some(check(
            FilterName::Keywords,
            pref.mode,
            Verdict::Fail,
            format!("mentions \"{bad}\""),
        ));
    }
    if pref.require_any.is_empty() {
        return Some(check(
            FilterName::Keywords,
            pref.mode,
            Verdict::Pass,
            "no excluded keywords",
        ));
    }
    Some(
        match pref.require_any.iter().find(|k| contains_phrase(&text, k)) {
            Some(hit) => check(
                FilterName::Keywords,
                pref.mode,
                Verdict::Pass,
                format!("mentions \"{hit}\""),
            ),
            None => check(
                FilterName::Keywords,
                pref.mode,
                Verdict::Fail,
                "mentions none of your required keywords",
            ),
        },
    )
}

fn company(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let pref = &ctx.search.companies;
    let name = normalize(&s.posting.company_name);
    let same = |other: &String| normalize(other) == name;
    if pref.block.iter().any(same) {
        return Some(check(
            FilterName::Company,
            Mode::Hard,
            Verdict::Fail,
            "company is on your block list",
        ));
    }
    if pref.exclude_staffing_agencies && s.staffing_agency == Some(true) {
        return Some(check(
            FilterName::Company,
            Mode::Hard,
            Verdict::Fail,
            "staffing agency",
        ));
    }
    if pref.allow_mode == Mode::Any || pref.allow.is_empty() {
        return None;
    }
    Some(if pref.allow.iter().any(same) {
        check(
            FilterName::Company,
            pref.allow_mode,
            Verdict::Pass,
            "on your allow list",
        )
    } else {
        check(
            FilterName::Company,
            pref.allow_mode,
            Verdict::Fail,
            "not on your allow list",
        )
    })
}

fn sponsorship(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let mode = ctx.search.requirements.sponsorship;
    if mode == Mode::Any {
        return None;
    }
    let Some(fact) = &s.posting.facts.sponsorship else {
        return Some(check(
            FilterName::Sponsorship,
            mode,
            Verdict::Pass,
            "not mentioned",
        ));
    };
    let evidence = &fact.evidence;
    Some(match fact.value {
        Sponsorship::Offered => check(
            FilterName::Sponsorship,
            mode,
            Verdict::Pass,
            "offers sponsorship",
        ),
        Sponsorship::NotOffered => match ctx.banks.needs_sponsorship() {
            Some(true) => check(
                FilterName::Sponsorship,
                mode,
                Verdict::Fail,
                format!("does not sponsor: \"{evidence}\""),
            ),
            Some(false) => check(
                FilterName::Sponsorship,
                mode,
                Verdict::Pass,
                "does not sponsor; you do not need it",
            ),
            None => check(
                FilterName::Sponsorship,
                mode,
                Verdict::Unknown,
                "does not sponsor; your sponsorship answer is not verified",
            ),
        },
        Sponsorship::CitizenshipRequired => match ctx.banks.us_citizen() {
            Some(true) => check(
                FilterName::Sponsorship,
                mode,
                Verdict::Pass,
                "requires US citizenship",
            ),
            Some(false) => check(
                FilterName::Sponsorship,
                mode,
                Verdict::Fail,
                format!("requires US citizenship: \"{evidence}\""),
            ),
            None => check(
                FilterName::Sponsorship,
                mode,
                Verdict::Unknown,
                "requires US citizenship; your citizenship answer is not verified",
            ),
        },
    })
}

fn clearance(s: &Subject<'_>, ctx: &FilterContext<'_>) -> Option<Check> {
    let mode = ctx.search.requirements.clearance;
    if mode == Mode::Any {
        return None;
    }
    let fact = s.posting.facts.clearance.as_ref()?;
    let req = fact.value;
    let need = format!("requires {:?} clearance", req.level);
    let Some(held) = ctx.banks.held_clearance() else {
        return Some(check(
            FilterName::Clearance,
            mode,
            Verdict::Unknown,
            format!("{need}; your clearance answer is not verified"),
        ));
    };
    Some(if held >= req.level {
        check(FilterName::Clearance, mode, Verdict::Pass, need)
    } else if !req.active_required && ctx.banks.willing_to_obtain_clearance() == Some(true) {
        check(
            FilterName::Clearance,
            mode,
            Verdict::Pass,
            format!("{need}, obtainable after hire"),
        )
    } else {
        check(
            FilterName::Clearance,
            mode,
            Verdict::Fail,
            format!("{need}: \"{}\"", fact.evidence),
        )
    })
}

fn not_checked_yet(filter: FilterName, mode: Mode) -> Option<Check> {
    (mode != Mode::Any).then(|| {
        check(
            filter,
            mode,
            Verdict::Unknown,
            "not checked yet: needs posting extraction",
        )
    })
}

#[cfg(test)]
mod tests;
