//! Option resolution (design §5, "set"): maps what a user or agent typed to
//! an enum option key or a bool, using the definition's
//! `options_normalization`. Deterministic; no fuzzy matching.

use std::collections::BTreeSet;

use crate::text::normalize_label;

use super::{AnswerType, QuestionDef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// An enum option key.
    Key(String),
    Bool(bool),
    /// The bool question's `decline` key.
    Decline,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OptionError {
    #[error("ambiguous: matches {}", .0.join(", "))]
    Ambiguous(Vec<String>),
    #[error("unknown option; valid: {}", .valid.join(", "))]
    Unknown { valid: Vec<String> },
    #[error("question is not a choice")]
    NotChoice,
}

/// Resolve `input` for an enum or bool definition.
///
/// Enum: an exact key wins; otherwise the input is compared, after
/// [`normalize_label`], with each key and each of its labels. Bool: the
/// built-in `yes`/`no`/`true`/`false` plus the definition's labels; the
/// `decline` key yields [`Resolved::Decline`].
pub fn resolve_option(def: &QuestionDef, input: &str) -> Result<Resolved, OptionError> {
    match def.answer_type {
        AnswerType::Enum => {
            if def.options_normalization.contains_key(input) {
                return Ok(Resolved::Key(input.to_owned()));
            }
            let keys = matching_keys(def, input, &[]);
            pick(keys, def, |k| Resolved::Key(k.to_owned()))
        }
        AnswerType::Bool => {
            let builtin = [
                ("yes", "yes"),
                ("true", "yes"),
                ("no", "no"),
                ("false", "no"),
            ];
            let keys = matching_keys(def, input, &builtin);
            pick(keys, def, |k| match k {
                "yes" => Resolved::Bool(true),
                "no" => Resolved::Bool(false),
                _ => Resolved::Decline,
            })
        }
        _ => Err(OptionError::NotChoice),
    }
}

/// Keys whose name or labels (or the extra `(text, key)` pairs) normalize to
/// the same text as `input`.
fn matching_keys(def: &QuestionDef, input: &str, extra: &[(&str, &str)]) -> BTreeSet<String> {
    let want = normalize_label(input);
    let mut keys = BTreeSet::new();
    if want.is_empty() {
        return keys;
    }
    for (text, key) in extra {
        if normalize_label(text) == want {
            keys.insert((*key).to_owned());
        }
    }
    for (key, labels) in &def.options_normalization {
        let hit = normalize_label(key) == want || labels.iter().any(|l| normalize_label(l) == want);
        if hit {
            keys.insert(key.clone());
        }
    }
    keys
}

fn pick(
    keys: BTreeSet<String>,
    def: &QuestionDef,
    make: impl Fn(&str) -> Resolved,
) -> Result<Resolved, OptionError> {
    let mut it = keys.iter();
    match (it.next(), it.next()) {
        (Some(k), None) => Ok(make(k)),
        (Some(_), Some(_)) => Err(OptionError::Ambiguous(keys.into_iter().collect())),
        _ => {
            let mut valid: Vec<String> = def.options_normalization.keys().cloned().collect();
            if def.answer_type == AnswerType::Bool {
                for k in ["yes", "no"] {
                    if !valid.iter().any(|v| v == k) {
                        valid.push(k.to_owned());
                    }
                }
                valid.sort();
            }
            Err(OptionError::Unknown { valid })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(toml_extra: &str) -> QuestionDef {
        toml::from_str(&format!(
            "id='legal.x'\nsection='C'\nlabel='x'\nsensitivity='legal'\n{toml_extra}"
        ))
        .unwrap()
    }

    fn enum_def() -> QuestionDef {
        def("answer_type='enum'\n\
             options=['us_citizen','green_card','other','decline']\n\
             options_normalization={ us_citizen=['U.S. Citizen','Citizen'], \
             green_card=['Permanent Resident','Citizen or Resident'], other=['Other'], \
             decline=['Prefer not to answer'] }")
    }

    fn bool_def() -> QuestionDef {
        def("answer_type='bool'\n\
             options_normalization={ yes=['Yes, I do'], no=['No, I do not'], \
             decline=['Prefer not to answer'] }")
    }

    fn key(s: &str) -> Result<Resolved, OptionError> {
        Ok(Resolved::Key(s.to_owned()))
    }

    #[test]
    fn enum_exact_key_and_label() {
        let d = enum_def();
        assert_eq!(resolve_option(&d, "green_card"), key("green_card"));
        assert_eq!(resolve_option(&d, "Permanent Resident"), key("green_card"));
    }

    #[test]
    fn enum_label_ignores_case_whitespace_and_trailing_star() {
        let d = enum_def();
        assert_eq!(resolve_option(&d, "  u.s.  CITIZEN *"), key("us_citizen"));
        assert_eq!(resolve_option(&d, "other:"), key("other"));
        assert_eq!(resolve_option(&d, "Green_Card"), key("green_card"));
    }

    #[test]
    fn enum_decline_is_an_ordinary_key() {
        let d = enum_def();
        assert_eq!(resolve_option(&d, "prefer not to answer"), key("decline"));
    }

    #[test]
    fn enum_ambiguous_and_unknown() {
        let d = enum_def();
        assert_eq!(resolve_option(&d, "citizen or resident"), key("green_card"));
        let mut amb = d.clone();
        amb.options_normalization
            .get_mut("us_citizen")
            .unwrap()
            .push("Citizen or Resident".into());
        assert_eq!(
            resolve_option(&amb, "citizen or resident"),
            Err(OptionError::Ambiguous(vec![
                "green_card".into(),
                "us_citizen".into()
            ]))
        );
        assert_eq!(
            resolve_option(&d, "martian"),
            Err(OptionError::Unknown {
                valid: vec![
                    "decline".into(),
                    "green_card".into(),
                    "other".into(),
                    "us_citizen".into()
                ]
            })
        );
        assert!(matches!(
            resolve_option(&d, "  *"),
            Err(OptionError::Unknown { .. })
        ));
    }

    #[test]
    fn bool_forms() {
        let d = bool_def();
        for s in ["yes", "Yes", "TRUE", " true ", "Yes, I do"] {
            assert_eq!(resolve_option(&d, s), Ok(Resolved::Bool(true)), "{s}");
        }
        for s in ["no", "False", "No, I do not *"] {
            assert_eq!(resolve_option(&d, s), Ok(Resolved::Bool(false)), "{s}");
        }
        assert!(matches!(
            resolve_option(&d, "maybe"),
            Err(OptionError::Unknown { valid }) if valid == ["decline", "no", "yes"]
        ));
    }

    #[test]
    fn bool_decline() {
        let d = bool_def();
        assert_eq!(resolve_option(&d, "decline"), Ok(Resolved::Decline));
        assert_eq!(
            resolve_option(&d, "Prefer not to answer"),
            Ok(Resolved::Decline)
        );
        let plain = def("answer_type='bool'");
        assert!(matches!(
            resolve_option(&plain, "decline"),
            Err(OptionError::Unknown { .. })
        ));
        assert_eq!(resolve_option(&plain, "yes"), Ok(Resolved::Bool(true)));
    }

    #[test]
    fn non_choice_types() {
        for t in ["text", "date", "number", "list", "file", "grid"] {
            let d = def(&format!("answer_type='{t}'"));
            assert_eq!(resolve_option(&d, "x"), Err(OptionError::NotChoice));
        }
    }
}
