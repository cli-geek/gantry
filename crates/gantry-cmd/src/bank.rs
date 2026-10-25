//! `gantry bank list` and `gantry bank set` (m2a question-bank design §5,
//! §7). Bank files are plain TOML tables; a set rewrites one file through
//! `update_table`, so keys this version does not know survive.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use gantry_core::questions::{
    AnswerState, AnswerType, Catalog, CoverageReport, Derived, OptionError, PolicyMode,
    QuestionDef, Resolved, Section, Sensitivity, Stored, ValueWarning, composable, coverage,
    derive, resolve_option, stored_at, unknown_paths, validate_value, value_at,
};
use gantry_store::{BankDocs, StoreError, bundled_catalog, load_bank_docs, update_table};
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::Serialize;

use crate::{CmdError, Context};

/// Which questions `bank_list` returns. `None` matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BankFilter {
    pub section: Option<Section>,
    /// A pack name, or `core` for the questions outside every pack.
    pub pack: Option<String>,
    pub state: Option<AnswerState>,
    pub sensitivity: Option<Sensitivity>,
}

/// One question and what the bank holds for it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct BankRow {
    pub id: String,
    pub section: Section,
    pub label: String,
    pub sensitivity: Sensitivity,
    pub answer_type: AnswerType,
    /// `None` for core questions.
    pub pack: Option<String>,
    pub state: AnswerState,
    /// The stored value (a list as an array); `None` when nothing is stored
    /// or the value is redacted.
    pub value: Option<serde_json::Value>,
    /// A value is stored but hidden (legal, eeo, clearance and availability
    /// values need `values`).
    pub redacted: bool,
    /// Whether the stored value is verified (every entry, for a list);
    /// `None` when nothing is stored.
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct BankList {
    pub rows: Vec<BankRow>,
    /// Stored values that match no question; they are kept as they are.
    pub warnings: Vec<String>,
}

/// What `bank_set` was asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum SetAction {
    /// Scalar input as typed; parsed by the question's answer type.
    Value(String),
    /// A value as JSON; the only input for lists, grids and records.
    ValueJson(serde_json::Value),
    Decline,
    AlwaysAsk,
    NotApplicable,
    ClearPolicy,
}

/// A suggestion stored by the derive rules.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DerivedRow {
    pub id: String,
    /// The question whose new answer suggested it.
    pub from: String,
    pub value: Option<serde_json::Value>,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SetOutcome {
    pub id: String,
    /// `value`, `decline`, `always_ask`, `not_applicable` or `clear_policy`.
    pub action: String,
    /// The written item is verified; `false` for `clear_policy`, which
    /// writes nothing.
    pub verified: bool,
    /// The value written (`value` action only); `None` when redacted.
    pub value: Option<serde_json::Value>,
    pub redacted: bool,
    pub derived: Vec<DerivedRow>,
    /// A policy row was removed.
    pub policy_cleared: bool,
    pub warnings: Vec<String>,
}

/// Whether a value of this class is hidden unless the caller asks for it
/// (invariant 6: an agent reading stdout is an LLM).
fn is_redacted(sensitivity: Sensitivity) -> bool {
    matches!(
        sensitivity,
        Sensitivity::Legal | Sensitivity::Eeo | Sensitivity::Clearance | Sensitivity::Availability
    )
}

fn catalog() -> Result<Catalog, CmdError> {
    bundled_catalog().map_err(|e| {
        CmdError::Config(StoreError::Toml {
            path: PathBuf::from("data/questions"),
            message: e.to_string(),
        })
    })
}

/// An unparsable file is a config error; other failures are I/O.
fn store_error(e: StoreError) -> CmdError {
    match e {
        StoreError::Toml { .. } => CmdError::Config(e),
        other => CmdError::Store(other),
    }
}

fn load_docs(ctx: &Context) -> Result<BankDocs, CmdError> {
    load_bank_docs(ctx.paths.config_dir()).map_err(store_error)
}

fn file_error(ctx: &Context, file: &str, message: String) -> CmdError {
    CmdError::Config(StoreError::Toml {
        path: ctx.paths.config_dir().join(file),
        message,
    })
}

fn stem(def: &QuestionDef) -> &str {
    def.id.split('.').next().unwrap_or_default()
}

fn to_json(value: &toml::Value) -> Option<serde_json::Value> {
    serde_json::to_value(value).ok()
}

/// `gantry bank list`: every addressable question that applies (core, plus
/// the packs named in `search.toml` `occupations`), optional ones included.
/// Values of sensitive questions are hidden unless `values`.
pub fn bank_list(ctx: &Context, filter: &BankFilter, values: bool) -> Result<BankList, CmdError> {
    let catalog = catalog()?;
    let docs = load_docs(ctx)?;
    let occupations = ctx.snapshot()?.search.occupations;
    let states = docs.states(&catalog);
    let mut rows = Vec::new();
    for def in catalog.iter() {
        if !def.is_addressable() {
            continue;
        }
        let pack = catalog.pack_of(&def.id);
        if pack.is_some_and(|p| !occupations.iter().any(|o| o == p)) {
            continue;
        }
        let mut state = states.get(&def.id).copied().unwrap_or(AnswerState::Missing);
        if state == AnswerState::Missing && composable(&catalog, def, &states) {
            state = AnswerState::Composable;
        }
        let wanted = filter.section.is_none_or(|s| s == def.section)
            && filter
                .pack
                .as_deref()
                .is_none_or(|p| p == pack.unwrap_or("core"))
            && filter.state.is_none_or(|s| s == state)
            && filter.sensitivity.is_none_or(|s| s == def.sensitivity);
        if !wanted {
            continue;
        }
        let stored = docs.files.get(stem(def)).and_then(|doc| value_at(doc, def));
        let hide = !values && is_redacted(def.sensitivity);
        let redacted = hide && stored.is_some();
        rows.push(BankRow {
            id: def.id.clone(),
            section: def.section,
            label: def.label.clone(),
            sensitivity: def.sensitivity,
            answer_type: def.answer_type,
            pack: pack.map(str::to_owned),
            state,
            value: match &stored {
                Some((v, _)) if !hide => to_json(v),
                _ => None,
            },
            redacted,
            verified: stored.map(|(_, verified)| verified),
        });
    }
    let warnings = docs
        .files
        .iter()
        .flat_map(|(file, doc)| unknown_paths(&catalog, file, doc))
        .map(|id| format!("{id} matches no question; it is kept as it is"))
        .collect();
    Ok(BankList { rows, warnings })
}

/// `gantry bank set`: saves a value (verified unless `unverified`) or sets
/// or clears a policy. Setting a value removes the question's policy row;
/// setting a policy keeps the stored value. A verified value also stores
/// derive suggestions for questions that have no answer yet.
///
/// The returned values of legal, eeo, clearance and availability questions
/// are hidden unless `values` (design §5).
///
/// Each file is written atomically on its own; the value file, `answers.toml`
/// and any derived files are separate writes. A rejected input changes
/// nothing. Errors: unknown or non-addressable id, or a value the question
/// rejects, are `InvalidInput`; an unparsable bank file is `Config`.
pub fn bank_set(
    ctx: &Context,
    id: &str,
    action: SetAction,
    unverified: bool,
    values: bool,
) -> Result<SetOutcome, CmdError> {
    let catalog = catalog()?;
    let def = catalog
        .get(id)
        .ok_or_else(|| CmdError::InvalidInput(format!("unknown question id {id:?}")))?;
    if !def.is_addressable() {
        return Err(CmdError::InvalidInput(format!(
            "{id} is an attestation or prohibited question; it has no address and takes no answer"
        )));
    }
    let docs = load_docs(ctx)?;
    let stamp = Timestamp::from_second(crate::now())
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_string();
    let hide = !values && is_redacted(def.sensitivity);
    let mut outcome = SetOutcome {
        id: id.to_owned(),
        action: String::new(),
        verified: true,
        value: None,
        redacted: false,
        derived: Vec::new(),
        policy_cleared: false,
        warnings: Vec::new(),
    };

    let mode = match &action {
        SetAction::Value(_) | SetAction::ValueJson(_) => None,
        SetAction::Decline => Some(PolicyMode::Decline),
        SetAction::AlwaysAsk => Some(PolicyMode::AlwaysAsk),
        SetAction::NotApplicable => Some(PolicyMode::NotApplicable),
        SetAction::ClearPolicy => {
            outcome.action = "clear_policy".to_owned();
            outcome.verified = false;
            outcome.policy_cleared = remove_policy(ctx, &docs, id)?;
            return Ok(outcome);
        }
    };
    if let Some(mode) = mode {
        let name = policy_name(mode);
        let file = "answers.toml";
        let path = ctx.paths.config_dir().join(file);
        update_table(&path, |t| write_policy(t, id, name, &stamp))
            .map_err(store_error)?
            .map_err(|m| file_error(ctx, file, m))?;
        outcome.action = name.to_owned();
        return Ok(outcome);
    }

    let value = parse_input(def, action)?;
    let warnings =
        validate_value(def, &value).map_err(|e| CmdError::InvalidInput(format!("{id}: {e}")))?;
    outcome.warnings = warnings.iter().map(warning_text).collect();
    let verified = !unverified;
    let file = format!("{}.toml", stem(def));
    // Check the address is writable before touching the file.
    let mut probe = docs.files.get(stem(def)).cloned().unwrap_or_default();
    write_item(&mut probe, def, &value, verified, &stamp, None)
        .map_err(|m| file_error(ctx, &file, m))?;
    let path = ctx.paths.config_dir().join(&file);
    update_table(&path, |t| {
        write_item(t, def, &value, verified, &stamp, None)
    })
    .map_err(store_error)?
    .map_err(|m| file_error(ctx, &file, m))?;
    outcome.action = "value".to_owned();
    outcome.verified = verified;
    outcome.redacted = hide;
    outcome.value = if hide { None } else { to_json(&value) };
    outcome.policy_cleared = remove_policy(ctx, &docs, id)?;
    if verified {
        outcome.derived = store_derived(ctx, &catalog, &stamp, values)?;
    }
    Ok(outcome)
}

fn policy_name(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Decline => "decline",
        PolicyMode::AlwaysAsk => "always_ask",
        PolicyMode::NotApplicable => "not_applicable",
    }
}

fn warning_text(w: &ValueWarning) -> String {
    match w {
        ValueWarning::UnknownField { path, key } => {
            let at = if path.is_empty() { "value" } else { path };
            format!("{at}: {key:?} is not a field of this question; kept")
        }
    }
}

/// Turn what the user typed into a TOML value, per the answer type.
fn parse_input(def: &QuestionDef, action: SetAction) -> Result<toml::Value, CmdError> {
    let invalid = |m: String| CmdError::InvalidInput(format!("{}: {m}", def.id));
    let text = match action {
        SetAction::ValueJson(json) => {
            return toml::Value::try_from(json)
                .map_err(|e| invalid(format!("not a TOML value: {e}")));
        }
        SetAction::Value(text) => text,
        _ => return Err(invalid("no value to parse".to_owned())),
    };
    match def.answer_type {
        AnswerType::Bool | AnswerType::Enum => match resolve_option(def, &text) {
            Ok(Resolved::Bool(b)) => Ok(toml::Value::Boolean(b)),
            Ok(Resolved::Key(key)) => Ok(toml::Value::String(key)),
            Ok(Resolved::Decline) => Err(invalid(format!(
                "{text:?} means declining; use the decline action"
            ))),
            Err(e @ (OptionError::Ambiguous(_) | OptionError::Unknown { .. })) => {
                Err(invalid(format!("{text:?}: {e}")))
            }
            Err(e) => Err(invalid(e.to_string())),
        },
        AnswerType::Number => {
            let t = text.trim();
            t.parse::<i64>()
                .map(toml::Value::Integer)
                .or_else(|_| t.parse::<f64>().map(toml::Value::Float))
                .map_err(|_| invalid(format!("{text:?} is not a number")))
        }
        AnswerType::Date | AnswerType::Text | AnswerType::File => Ok(toml::Value::String(text)),
        AnswerType::List | AnswerType::Grid => Err(invalid(
            "this answer is a list or grid; give it as JSON".to_owned(),
        )),
    }
}

/// The parent table of `id`'s address in `doc` (created on the way) and the
/// last path segment.
fn slot<'a>(
    doc: &'a mut toml::Table,
    id: &'a str,
) -> Result<(&'a mut toml::Table, &'a str), String> {
    let segments: Vec<&str> = id.split('.').skip(1).collect();
    let (last, parents) = segments
        .split_last()
        .ok_or_else(|| format!("{id} has no path"))?;
    let mut table = doc;
    for seg in parents {
        table = table
            .entry(*seg)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("{seg:?} must be a table to hold {id}"))?;
    }
    Ok((table, last))
}

/// Writes `value` for `def` into its file's table. A scalar updates its item
/// in place (keeping `created_at`, an existing `id` and unknown keys); a
/// list is replaced by one entry per element, ids `<last segment>-<n>`.
/// `source` marks a derived suggestion.
fn write_item(
    doc: &mut toml::Table,
    def: &QuestionDef,
    value: &toml::Value,
    verified: bool,
    stamp: &str,
    source: Option<&str>,
) -> Result<(), String> {
    let (parent, last) = slot(doc, &def.id)?;
    if def.answer_type == AnswerType::List {
        let elements = value.as_array().ok_or("expected an array")?;
        let rows = elements
            .iter()
            .enumerate()
            .map(|(i, element)| {
                let mut row = toml::Table::new();
                row.insert("id".into(), format!("{last}-{}", i + 1).into());
                row.insert("value".into(), element.clone());
                row.insert("verified".into(), verified.into());
                row.insert("created_at".into(), stamp.into());
                row.insert("updated_at".into(), stamp.into());
                if let Some(source) = source {
                    row.insert("source".into(), source.into());
                }
                toml::Value::Table(row)
            })
            .collect();
        parent.insert(last.to_owned(), toml::Value::Array(rows));
        return Ok(());
    }
    let item = parent
        .entry(last)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| format!("{} must be a table", def.id))?;
    item.entry("id").or_insert_with(|| def.id.clone().into());
    item.entry("created_at").or_insert_with(|| stamp.into());
    item.insert("value".into(), value.clone());
    item.insert("verified".into(), verified.into());
    item.insert("updated_at".into(), stamp.into());
    match source {
        Some(source) => item.insert("source".into(), source.into()),
        None => item.remove("source"),
    };
    Ok(())
}

fn write_policy(
    answers: &mut toml::Table,
    id: &str,
    mode: &str,
    stamp: &str,
) -> Result<(), String> {
    let rows = answers
        .entry("policy")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("policy must be a table of items")?;
    let item = rows
        .entry(id)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| format!("policy.\"{id}\" must be a table"))?;
    item.entry("id").or_insert_with(|| id.into());
    item.entry("created_at").or_insert_with(|| stamp.into());
    item.insert("value".into(), mode.into());
    item.insert("verified".into(), true.into());
    item.insert("updated_at".into(), stamp.into());
    Ok(())
}

/// Removes `id`'s policy row if `docs` saw one; never creates the file.
fn remove_policy(ctx: &Context, docs: &BankDocs, id: &str) -> Result<bool, CmdError> {
    if !docs.policy.contains_key(id) {
        return Ok(false);
    }
    let path = ctx.paths.config_dir().join("answers.toml");
    update_table(&path, |t| {
        t.get_mut("policy")
            .and_then(toml::Value::as_table_mut)
            .is_some_and(|rows| rows.remove(id).is_some())
    })
    .map_err(store_error)
}

/// Stores unverified suggestions from the verified answers now on disk,
/// one write per file. Derive skips questions that already hold a value or
/// a policy, so nothing is overwritten.
fn store_derived(
    ctx: &Context,
    catalog: &Catalog,
    stamp: &str,
    values: bool,
) -> Result<Vec<DerivedRow>, CmdError> {
    let docs = load_docs(ctx)?;
    let occupations = ctx.snapshot()?.search.occupations;
    let mut verified = BTreeMap::new();
    let mut occupied = BTreeSet::new();
    for def in catalog.iter() {
        let doc = docs.files.get(stem(def));
        if doc.and_then(|d| stored_at(d, def)).is_some() || docs.policy.contains_key(&def.id) {
            occupied.insert(def.id.clone());
        }
        // A policy wins over the stored value, so it is not a source.
        if def.answer_type.is_scalar()
            && !docs.policy.contains_key(&def.id)
            && let Some((value, true)) = doc.and_then(|d| value_at(d, def))
        {
            verified.insert(def.id.clone(), value);
        }
    }
    let suggestions = derive(catalog, &occupations, &verified, &occupied);
    let mut by_file: BTreeMap<&str, Vec<(&QuestionDef, &Derived)>> = BTreeMap::new();
    for d in &suggestions {
        if let Some(def) = catalog.get(&d.id) {
            by_file.entry(stem(def)).or_default().push((def, d));
        }
    }
    for (stem, items) in by_file {
        let file = format!("{stem}.toml");
        let path = ctx.paths.config_dir().join(&file);
        update_table(&path, |t| {
            items.iter().try_for_each(|(def, d)| {
                write_item(t, def, &d.value, false, stamp, Some(&d.source()))
            })
        })
        .map_err(store_error)?
        .map_err(|m| file_error(ctx, &file, m))?;
    }
    Ok(suggestions
        .iter()
        .map(|d| {
            let hide = !values
                && catalog
                    .get(&d.id)
                    .is_some_and(|def| is_redacted(def.sensitivity));
            DerivedRow {
                id: d.id.clone(),
                from: d.from.clone(),
                value: if hide { None } else { to_json(&d.value) },
                redacted: hide,
            }
        })
        .collect())
}

/// What `bank_verify` did with each id.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct VerifyOutcome {
    /// Ids whose stored item (every entry, for a list) is now verified.
    pub verified: Vec<String>,
    /// Ids that were verified already.
    pub already: Vec<String>,
    /// Ids that are valid but hold no stored value.
    pub nothing_stored: Vec<String>,
    /// Ids that are not questions, or are not addressable.
    pub unknown: Vec<String>,
    /// Suggestions stored because a newly verified value could suggest more.
    pub derived: Vec<DerivedRow>,
}

/// One suggestion waiting for the user to verify it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PendingItem {
    pub id: String,
    pub label: String,
    pub section: Section,
    pub sensitivity: Sensitivity,
    /// The stored value; `None` when redacted.
    pub value: Option<serde_json::Value>,
    pub redacted: bool,
    /// Where the suggestion came from, e.g. `derived:<question id>`.
    pub source: Option<String>,
}

/// A question the user told gantry to ask every time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct AlwaysAskRow {
    pub id: String,
    pub label: String,
    pub section: Section,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct BankCoverage {
    pub report: CoverageReport,
    /// Applicable questions that "will always ask you", sorted by id.
    pub always_ask: Vec<AlwaysAskRow>,
}

/// The stored item(s) of `def` in `doc`, mutable, without creating anything.
fn leaf_mut<'a>(doc: &'a mut toml::Table, def: &QuestionDef) -> Option<&'a mut toml::Value> {
    let mut segments = def.id.split('.').skip(1).peekable();
    let mut table = doc;
    while let Some(seg) = segments.next() {
        let value = table.get_mut(seg)?;
        if segments.peek().is_none() {
            return Some(value);
        }
        table = value.as_table_mut()?;
    }
    None
}

/// Marks one item table verified and bumps `updated_at`.
fn verify_item(item: &mut toml::Value, stamp: &str) {
    if let Some(item) = item.as_table_mut() {
        item.insert("verified".into(), true.into());
        item.insert("updated_at".into(), stamp.into());
    }
}

/// Applicable (core, or in a pack named by `occupations`) addressable
/// questions.
fn applicable<'a>(
    catalog: &'a Catalog,
    occupations: &'a [String],
) -> impl Iterator<Item = &'a QuestionDef> {
    catalog.iter().filter(move |def| {
        def.is_addressable()
            && catalog
                .pack_of(&def.id)
                .is_none_or(|p| occupations.iter().any(|o| o == p))
    })
}

/// `gantry bank verify ID...`: flips stored items to verified. A list
/// question verifies every entry. Unknown or non-addressable ids are
/// reported in `unknown` and do not stop the batch; only a batch with no
/// valid id is an error (`InvalidInput`). A verified value can suggest
/// more, so derive runs afterwards. A `derived:` source tag is kept.
///
/// Each touched file is written once; derived files are separate writes.
/// Errors: an unparsable bank file is `Config`.
pub fn bank_verify(ctx: &Context, ids: &[String], values: bool) -> Result<VerifyOutcome, CmdError> {
    let catalog = catalog()?;
    let docs = load_docs(ctx)?;
    let stamp = Timestamp::from_second(crate::now())
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_string();
    let mut outcome = VerifyOutcome {
        verified: Vec::new(),
        already: Vec::new(),
        nothing_stored: Vec::new(),
        unknown: Vec::new(),
        derived: Vec::new(),
    };
    let mut seen = BTreeSet::new();
    let mut by_file: BTreeMap<&str, Vec<&QuestionDef>> = BTreeMap::new();
    for id in ids {
        if !seen.insert(id.as_str()) {
            continue;
        }
        let Some(def) = catalog.get(id).filter(|d| d.is_addressable()) else {
            outcome.unknown.push(id.clone());
            continue;
        };
        match docs.files.get(stem(def)).and_then(|d| stored_at(d, def)) {
            None | Some(Stored::List { items: 0, .. }) => outcome.nothing_stored.push(id.clone()),
            Some(Stored::Scalar { verified: true }) => outcome.already.push(id.clone()),
            Some(Stored::List { items, verified }) if verified == items => {
                outcome.already.push(id.clone());
            }
            Some(_) => {
                by_file.entry(stem(def)).or_default().push(def);
                outcome.verified.push(id.clone());
            }
        }
    }
    if outcome.unknown.len() == seen.len() && !seen.is_empty() {
        return Err(CmdError::InvalidInput(format!(
            "unknown question id {:?}",
            outcome.unknown[0]
        )));
    }
    for (stem, defs) in by_file {
        let file = format!("{stem}.toml");
        let path = ctx.paths.config_dir().join(&file);
        update_table(&path, |t| {
            for def in defs {
                match leaf_mut(t, def) {
                    Some(toml::Value::Array(rows)) => {
                        for row in rows {
                            verify_item(row, &stamp);
                        }
                    }
                    Some(item) => verify_item(item, &stamp),
                    None => {}
                }
            }
        })
        .map_err(store_error)?;
    }
    if !outcome.verified.is_empty() {
        outcome.derived = store_derived(ctx, &catalog, &stamp, values)?;
    }
    Ok(outcome)
}

/// `gantry bank verify` without ids: the suggestions waiting for the user,
/// intake order first (unranked last), then by id. Values of sensitive
/// questions are hidden unless `values`.
pub fn bank_pending(ctx: &Context, values: bool) -> Result<Vec<PendingItem>, CmdError> {
    let catalog = catalog()?;
    let docs = load_docs(ctx)?;
    let occupations = ctx.snapshot()?.search.occupations;
    let states = docs.states(&catalog);
    let mut pending: Vec<&QuestionDef> = applicable(&catalog, &occupations)
        .filter(|def| states.get(&def.id) == Some(&AnswerState::Suggested))
        .collect();
    pending.sort_by(|a, b| {
        let rank = |d: &QuestionDef| d.intake_rank.map_or((1, 0), |r| (0, r));
        rank(a).cmp(&rank(b)).then_with(|| a.id.cmp(&b.id))
    });
    Ok(pending
        .into_iter()
        .map(|def| {
            let doc = docs.files.get(stem(def));
            let stored = doc.and_then(|d| value_at(d, def));
            let hide = !values && is_redacted(def.sensitivity);
            PendingItem {
                id: def.id.clone(),
                label: def.label.clone(),
                section: def.section,
                sensitivity: def.sensitivity,
                value: match &stored {
                    Some((v, _)) if !hide => to_json(v),
                    _ => None,
                },
                redacted: hide && stored.is_some(),
                source: doc.and_then(|d| source_of(d, def)),
            }
        })
        .collect())
}

/// The `source` tag of `def`'s item (first entry, for a list).
fn source_of(doc: &toml::Table, def: &QuestionDef) -> Option<String> {
    let mut table = doc;
    let mut segments = def.id.split('.').skip(1).peekable();
    while let Some(seg) = segments.next() {
        let value = table.get(seg)?;
        if segments.peek().is_some() {
            table = value.as_table()?;
            continue;
        }
        let item = match value {
            toml::Value::Array(rows) => rows.iter().find_map(|r| r.as_table()?.get("source")),
            other => other.as_table()?.get("source"),
        };
        return item?.as_str().map(str::to_owned);
    }
    None
}

/// `gantry bank verify --all`: verifies every pending suggestion. `confirmed`
/// is the typed confirmation the CLI collected; without it nothing happens.
/// An accident guard, not a security boundary (design §5).
pub fn bank_verify_all(
    ctx: &Context,
    confirmed: bool,
    values: bool,
) -> Result<VerifyOutcome, CmdError> {
    if !confirmed {
        return Err(CmdError::InvalidInput(
            "verify --all needs a typed confirmation".to_owned(),
        ));
    }
    let ids: Vec<String> = bank_pending(ctx, values)?
        .into_iter()
        .map(|p| p.id)
        .collect();
    bank_verify(ctx, &ids, values)
}

/// `gantry bank coverage`: the meter over the applicable non-optional
/// questions, plus the questions the user set to always ask. No values
/// appear in the output.
pub fn bank_coverage(ctx: &Context) -> Result<BankCoverage, CmdError> {
    let catalog = catalog()?;
    let docs = load_docs(ctx)?;
    let occupations = ctx.snapshot()?.search.occupations;
    let states = docs.states(&catalog);
    let report = coverage(&catalog, &occupations, &states);
    let mut always_ask: Vec<AlwaysAskRow> = applicable(&catalog, &occupations)
        .filter(|def| states.get(&def.id) == Some(&AnswerState::AlwaysAsk))
        .map(|def| AlwaysAskRow {
            id: def.id.clone(),
            label: def.label.clone(),
            section: def.section,
        })
        .collect();
    always_ask.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(BankCoverage { report, always_ask })
}
