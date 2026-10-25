//! Variant harvest (m2a question-bank design §8): turns Greenhouse
//! application-question payloads into raw material for bank `variants`.
//!
//! Endpoint: `GET https://boards-api.greenhouse.io/v1/boards/{token}/jobs/{id}?questions=true`,
//! no authentication. Checked live 2026-10-24: besides the job fields it
//! carries four question groups, any of which may be `null`:
//!
//! - `questions`: the application form. Each item has `label`, `required`
//!   and `fields: [{name, type, values: [{label, value}]}]`; `type` is
//!   `input_text`, `textarea`, `input_file`, `multi_value_single_select` or
//!   `multi_value_multi_select`.
//! - `location_questions`: the same shape; the Longitude, Latitude and
//!   Location items are `input_hidden`.
//! - `compliance`: a list of `{type: "eeoc", questions: [...]}` groups with
//!   the same item shape (Race, Gender, VeteranStatus, ...).
//! - `demographic_questions`: an object `{header, description, questions}`
//!   whose items differ: `type` sits on the item and the options are
//!   `answer_options: [{label, ...}]` instead of `fields`.
//!
//! Pure functions only; the network driver is
//! `examples/harvest_questions.rs`. The output is reviewed by a human and
//! copied into `data/questions/` by a PR; nothing here writes there.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use gantry_core::text::normalize_label;

use crate::sources::ParseError;

const SOURCE: &str = "greenhouse questions";

/// Which part of the payload a question came from. Questions with the same
/// wording in different groups are different questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Questions,
    LocationQuestions,
    Compliance,
    DemographicQuestions,
}

/// One question as one job presents it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarvestedQuestion {
    pub label: String,
    pub group: Group,
    /// Raw Greenhouse type strings, sorted and deduplicated.
    pub field_types: Vec<String>,
    /// Select option labels, sorted and deduplicated.
    pub options: Vec<String>,
    pub required: bool,
}

#[derive(Deserialize)]
struct Job {
    questions: Option<Vec<Item>>,
    location_questions: Option<Vec<Item>>,
    compliance: Option<Vec<Compliance>>,
    demographic_questions: Option<Demographic>,
}

#[derive(Deserialize)]
struct Compliance {
    questions: Option<Vec<Item>>,
}

#[derive(Deserialize)]
struct Demographic {
    questions: Option<Vec<Item>>,
}

#[derive(Deserialize)]
struct Item {
    label: Option<String>,
    required: Option<bool>,
    /// Demographic items carry their type here rather than in `fields`.
    #[serde(rename = "type")]
    kind: Option<String>,
    fields: Option<Vec<Field>>,
    answer_options: Option<Vec<LabelOnly>>,
}

#[derive(Deserialize)]
struct Field {
    #[serde(rename = "type")]
    kind: Option<String>,
    values: Option<Vec<LabelOnly>>,
}

#[derive(Deserialize)]
struct LabelOnly {
    label: Option<String>,
}

fn item_to_question(item: Item, group: Group) -> Option<HarvestedQuestion> {
    let label = item.label?.trim().to_owned();
    if label.is_empty() {
        return None;
    }
    let mut types: BTreeSet<String> = item.kind.into_iter().collect();
    let mut options = BTreeSet::new();
    for field in item.fields.into_iter().flatten() {
        types.extend(field.kind);
        options.extend(labels(field.values));
    }
    options.extend(labels(item.answer_options));
    Some(HarvestedQuestion {
        label,
        group,
        field_types: types.into_iter().collect(),
        options: options.into_iter().collect(),
        required: item.required.unwrap_or(false),
    })
}

fn labels(values: Option<Vec<LabelOnly>>) -> impl Iterator<Item = String> {
    values
        .into_iter()
        .flatten()
        .filter_map(|v| v.label)
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
}

/// Every question in the response of a `?questions=true` job request, in
/// payload order: `questions`, `location_questions`, `compliance`,
/// `demographic_questions`. Missing or `null` groups yield nothing.
pub fn parse_job_questions(body: &[u8]) -> Result<Vec<HarvestedQuestion>, ParseError> {
    let job: Job = serde_json::from_slice(body).map_err(|e| ParseError::json(SOURCE, &e))?;
    let groups = [
        (Group::Questions, job.questions.unwrap_or_default()),
        (
            Group::LocationQuestions,
            job.location_questions.unwrap_or_default(),
        ),
        (
            Group::Compliance,
            job.compliance
                .into_iter()
                .flatten()
                .flat_map(|c| c.questions.unwrap_or_default())
                .collect(),
        ),
        (
            Group::DemographicQuestions,
            job.demographic_questions
                .and_then(|d| d.questions)
                .unwrap_or_default(),
        ),
    ];
    Ok(groups
        .into_iter()
        .flat_map(|(group, items)| {
            items
                .into_iter()
                .filter_map(move |item| item_to_question(item, group))
        })
        .collect())
}

#[derive(Deserialize)]
struct BoardList {
    jobs: Vec<ListedJob>,
}

#[derive(Deserialize)]
struct ListedJob {
    id: u64,
}

/// The first `count` job ids of a board list body in ascending numeric
/// order, so repeated runs sample the same jobs while the board is stable.
pub fn pick_job_ids(body: &[u8], count: usize) -> Result<Vec<u64>, ParseError> {
    let list: BoardList = serde_json::from_slice(body).map_err(|e| ParseError::json(SOURCE, &e))?;
    let mut ids: Vec<u64> = list.jobs.into_iter().map(|j| j.id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.truncate(count);
    Ok(ids)
}

/// The questions one job presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSample {
    pub token: String,
    pub job_id: String,
    pub questions: Vec<HarvestedQuestion>,
}

/// One question merged across jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AggregatedQuestion {
    /// The wording of the first job that presented it.
    pub text: String,
    pub normalized: String,
    pub group: Group,
    pub ats: &'static str,
    pub field_types: Vec<String>,
    pub options: Vec<String>,
    pub boards: usize,
    pub jobs: usize,
    /// `"{token}/{job_id}"` of the first job that presented it.
    pub example: String,
}

struct Acc {
    text: String,
    field_types: BTreeSet<String>,
    options: BTreeSet<String>,
    boards: BTreeSet<String>,
    jobs: BTreeSet<(String, String)>,
    example: String,
}

/// Merges by (group, normalized label) across `samples`, in the order
/// given (that order decides the first-seen text and example). Sorted by
/// boards descending, jobs descending, then normalized label and group.
pub fn aggregate(samples: &[JobSample]) -> Vec<AggregatedQuestion> {
    let mut merged: BTreeMap<(Group, String), Acc> = BTreeMap::new();
    for sample in samples {
        for q in &sample.questions {
            let normalized = normalize_label(&q.label);
            if normalized.is_empty() {
                continue;
            }
            let acc = merged.entry((q.group, normalized)).or_insert_with(|| Acc {
                text: q.label.clone(),
                field_types: BTreeSet::new(),
                options: BTreeSet::new(),
                boards: BTreeSet::new(),
                jobs: BTreeSet::new(),
                example: format!("{}/{}", sample.token, sample.job_id),
            });
            acc.field_types.extend(q.field_types.iter().cloned());
            acc.options.extend(q.options.iter().cloned());
            acc.boards.insert(sample.token.clone());
            acc.jobs
                .insert((sample.token.clone(), sample.job_id.clone()));
        }
    }
    let mut out: Vec<AggregatedQuestion> = merged
        .into_iter()
        .map(|((group, normalized), acc)| AggregatedQuestion {
            text: acc.text,
            normalized,
            group,
            ats: "greenhouse",
            field_types: acc.field_types.into_iter().collect(),
            options: acc.options.into_iter().collect(),
            boards: acc.boards.len(),
            jobs: acc.jobs.len(),
            example: acc.example,
        })
        .collect();
    out.sort_by(|a, b| {
        (b.boards, b.jobs)
            .cmp(&(a.boards, a.jobs))
            .then_with(|| a.normalized.cmp(&b.normalized))
            .then_with(|| a.group.cmp(&b.group))
    });
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardFailure {
    pub token: String,
    pub error: String,
}

/// The file the example writes to `fixtures/questions/harvest-<date>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HarvestReport {
    /// `YYYY-MM-DD`.
    pub harvested_on: String,
    pub ats: &'static str,
    /// Boards with at least one parsed job.
    pub boards_sampled: usize,
    pub jobs_sampled: usize,
    pub boards_failed: Vec<BoardFailure>,
    pub questions: Vec<AggregatedQuestion>,
}

impl HarvestReport {
    pub fn new(
        harvested_on: String,
        samples: &[JobSample],
        boards_failed: Vec<BoardFailure>,
    ) -> Self {
        let boards: BTreeSet<&str> = samples.iter().map(|s| s.token.as_str()).collect();
        Self {
            harvested_on,
            ats: "greenhouse",
            boards_sampled: boards.len(),
            jobs_sampled: samples.len(),
            boards_failed,
            questions: aggregate(samples),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOUDFLARE: &[u8] =
        include_bytes!("../../../fixtures/questions/greenhouse/cloudflare-7695702.json");
    const COINBASE: &[u8] =
        include_bytes!("../../../fixtures/questions/greenhouse/coinbase-8175363.json");
    const DISCORD: &[u8] =
        include_bytes!("../../../fixtures/questions/greenhouse/discord-8806482002.json");
    const AIRBNB: &[u8] =
        include_bytes!("../../../fixtures/questions/greenhouse/airbnb-7789554.json");

    fn find<'a>(qs: &'a [HarvestedQuestion], group: Group, label: &str) -> &'a HarvestedQuestion {
        qs.iter()
            .find(|q| q.group == group && q.label == label)
            .unwrap_or_else(|| panic!("no {group:?} question {label:?}"))
    }

    fn count(qs: &[HarvestedQuestion], group: Group) -> usize {
        qs.iter().filter(|q| q.group == group).count()
    }

    fn sample(token: &str, id: &str, body: &[u8]) -> JobSample {
        JobSample {
            token: token.to_owned(),
            job_id: id.to_owned(),
            questions: parse_job_questions(body).unwrap(),
        }
    }

    #[test]
    fn every_fixture_parses() {
        for body in [CLOUDFLARE, COINBASE, DISCORD, AIRBNB] {
            let qs = parse_job_questions(body).unwrap();
            assert!(count(&qs, Group::Questions) >= 7);
            assert_eq!(count(&qs, Group::LocationQuestions), 3);
        }
    }

    #[test]
    fn compliance_questions_are_flattened() {
        let qs = parse_job_questions(CLOUDFLARE).unwrap();
        assert_eq!(count(&qs, Group::Compliance), 3);
        let race = find(&qs, Group::Compliance, "Race");
        assert_eq!(race.field_types, ["multi_value_single_select"]);
        assert!(!race.required);
        assert!(!race.options.is_empty());
        find(&qs, Group::Compliance, "VeteranStatus");
    }

    #[test]
    fn demographic_questions_use_item_type_and_answer_options() {
        let qs = parse_job_questions(DISCORD).unwrap();
        assert_eq!(count(&qs, Group::DemographicQuestions), 7);
        let gender = find(&qs, Group::DemographicQuestions, "Gender");
        assert!(gender.required);
        assert_eq!(gender.field_types, ["multi_value_single_select"]);
        assert!(gender.options.contains(&"Male".to_owned()));
        let race = find(
            &qs,
            Group::DemographicQuestions,
            "Race or Ethnicity (optional)",
        );
        assert_eq!(race.field_types, ["multi_value_multi_select"]);
        assert!(!race.required);
    }

    #[test]
    fn field_types_and_options_come_through() {
        let qs = parse_job_questions(CLOUDFLARE).unwrap();
        let resume = find(&qs, Group::Questions, "Resume/CV");
        assert_eq!(resume.field_types, ["input_file", "textarea"]);
        assert!(resume.required);
        let name = find(&qs, Group::Questions, "First Name");
        assert_eq!(name.field_types, ["input_text"]);
        assert!(name.options.is_empty());
        let sponsorship = qs
            .iter()
            .find(|q| {
                q.label
                    .starts_with("Do you now or will you in the future require")
            })
            .unwrap();
        assert_eq!(sponsorship.field_types, ["multi_value_single_select"]);
        assert_eq!(sponsorship.options, ["No", "Yes"]);
        let ack = qs
            .iter()
            .find(|q| q.label.starts_with("Please review and acknowledge"))
            .unwrap();
        assert_eq!(ack.field_types, ["multi_value_multi_select"]);
        let longitude = find(&qs, Group::LocationQuestions, "Longitude");
        assert_eq!(longitude.field_types, ["input_hidden"]);
    }

    #[test]
    fn null_groups_are_fine() {
        // Airbnb's job has null `compliance` and `demographic_questions`.
        let qs = parse_job_questions(AIRBNB).unwrap();
        assert_eq!(count(&qs, Group::Compliance), 0);
        assert_eq!(count(&qs, Group::DemographicQuestions), 0);
    }

    #[test]
    fn missing_groups_and_unknown_fields_are_fine() {
        assert!(parse_job_questions(b"{}").unwrap().is_empty());
        let body = br#"{"future": 1, "questions": [
            {"label": "Pronouns", "extra": true, "fields": null},
            {"label": "  ", "fields": []},
            {"required": true}],
            "compliance": [{"type": "eeoc"}], "demographic_questions": {"header": "x"}}"#;
        let qs = parse_job_questions(body).unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].label, "Pronouns");
        assert!(qs[0].field_types.is_empty());
        assert!(!qs[0].required);
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(parse_job_questions(b"{not json").is_err());
        assert!(parse_job_questions(br#"{"questions": "nope"}"#).is_err());
        assert!(pick_job_ids(b"[]", 3).is_err());
    }

    #[test]
    fn aggregate_merges_across_boards_and_keeps_groups_apart() {
        let q = |label: &str, group, types: &[&str], options: &[&str]| HarvestedQuestion {
            label: label.to_owned(),
            group,
            field_types: types.iter().map(|s| (*s).to_owned()).collect(),
            options: options.iter().map(|s| (*s).to_owned()).collect(),
            required: true,
        };
        let samples = [
            JobSample {
                token: "acme".into(),
                job_id: "1".into(),
                questions: vec![
                    q(
                        "Gender",
                        Group::Compliance,
                        &["multi_value_single_select"],
                        &["Male", "Female"],
                    ),
                    q(
                        "Gender:",
                        Group::DemographicQuestions,
                        &["multi_value_single_select"],
                        &[],
                    ),
                    q("Email", Group::Questions, &["input_text"], &[]),
                ],
            },
            JobSample {
                token: "zeta".into(),
                job_id: "2".into(),
                questions: vec![
                    q(
                        "GENDER *",
                        Group::Compliance,
                        &["input_text"],
                        &["Female", "Other"],
                    ),
                    q("Email", Group::Questions, &["input_text"], &[]),
                    q("email", Group::Questions, &["input_text"], &[]),
                ],
            },
        ];
        let agg = aggregate(&samples);
        assert_eq!(agg.len(), 3);
        // boards desc, jobs desc, then group and label.
        assert_eq!(agg[0].normalized, "email");
        assert_eq!(agg[1].normalized, "gender");
        assert_eq!(agg[1].group, Group::Compliance);
        assert_eq!(agg[2].group, Group::DemographicQuestions);

        let gender = &agg[1];
        assert_eq!(gender.text, "Gender");
        assert_eq!(gender.ats, "greenhouse");
        assert_eq!((gender.boards, gender.jobs), (2, 2));
        assert_eq!(
            gender.field_types,
            ["input_text", "multi_value_single_select"]
        );
        assert_eq!(gender.options, ["Female", "Male", "Other"]);
        assert_eq!(gender.example, "acme/1");

        // A repeat within one job counts once.
        assert_eq!((agg[0].boards, agg[0].jobs), (2, 2));
        assert_eq!((agg[2].boards, agg[2].jobs), (1, 1));
    }

    #[test]
    fn aggregate_counts_jobs_per_board() {
        let samples = [
            sample("cloudflare", "7695702", CLOUDFLARE),
            sample("cloudflare", "7695703", CLOUDFLARE),
            sample("coinbase", "8175363", COINBASE),
        ];
        let agg = aggregate(&samples);
        let email = agg.iter().find(|q| q.normalized == "email").unwrap();
        assert_eq!((email.boards, email.jobs), (2, 3));
        let privacy = agg
            .iter()
            .find(|q| q.normalized.starts_with("please review and acknowledge"))
            .unwrap();
        assert_eq!((privacy.boards, privacy.jobs), (1, 2));
        assert_eq!(aggregate(&samples), agg);
    }

    #[test]
    fn report_counts_and_serializes() {
        let samples = [
            sample("cloudflare", "1", CLOUDFLARE),
            sample("cloudflare", "2", CLOUDFLARE),
            sample("discord", "3", DISCORD),
        ];
        let failed = vec![BoardFailure {
            token: "gone".into(),
            error: "HTTP 404".into(),
        }];
        let report = HarvestReport::new("2026-10-24".into(), &samples, failed);
        assert_eq!((report.boards_sampled, report.jobs_sampled), (2, 3));
        let json: serde_json::Value = serde_json::to_value(&report).unwrap();
        assert_eq!(json["harvested_on"], "2026-10-24");
        assert_eq!(json["ats"], "greenhouse");
        assert_eq!(json["boards_failed"][0]["token"], "gone");
        assert_eq!(json["questions"][0]["ats"], "greenhouse");
        assert!(json["questions"][0]["group"].is_string());
    }

    #[test]
    fn job_ids_ascending_and_repeatable() {
        let body = br#"{"jobs": [{"id": 30}, {"id": 5}, {"id": 12}, {"id": 5}, {"id": 99}],
            "meta": {"total": 5}}"#;
        assert_eq!(pick_job_ids(body, 3).unwrap(), [5, 12, 30]);
        assert_eq!(pick_job_ids(body, 3).unwrap(), [5, 12, 30]);
        assert_eq!(pick_job_ids(body, 10).unwrap(), [5, 12, 30, 99]);
        assert!(pick_job_ids(br#"{"jobs": []}"#, 4).unwrap().is_empty());
    }
}
