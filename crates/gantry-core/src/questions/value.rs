//! Value validation (design §2, §5 "set"): checks that a bare TOML value has
//! the shape a question's answer type demands. The `Item` wrapper (verified
//! flag, id) is not part of the value and is not looked at here.
//!
//! Errors carry a path to the offending part: `""` for the root, `"[2]"` for
//! a list element, `"[2].start_date"` for a record field, `"mon[1]"` for a
//! grid range. Nothing here performs I/O.

use std::collections::BTreeSet;

use toml::Value;

use super::{AnswerType, FieldDef, QuestionDef};

/// Grid keys, in week order.
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// Why a value was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("question has no address and takes no value")]
    NotAddressable,
    #[error("{}: expected {expected}", at(.path))]
    TypeMismatch {
        path: String,
        expected: &'static str,
    },
    #[error("{}: not a valid YYYY-MM-DD date", at(.path))]
    BadDate { path: String },
    #[error("{}: native TOML datetime; use a \"YYYY-MM-DD\" string", at(.path))]
    NativeDatetime { path: String },
    #[error("{}: {value:?} is not an option; valid: {}", at(.path), .valid.join(", "))]
    BadOption {
        path: String,
        value: String,
        valid: Vec<String>,
    },
    #[error("{}: number must be finite", at(.path))]
    NotFinite { path: String },
    #[error("{}: must not be empty", at(.path))]
    Empty { path: String },
    #[error("{}: missing field {field:?}", at(.path))]
    MissingField { path: String, field: String },
    #[error("{}: {key:?} is not a day (mon..sun)", at(.path))]
    BadDay { path: String, key: String },
    #[error("{}: {value:?} is not HH:MM-HH:MM", at(.path))]
    BadRange { path: String, value: String },
    #[error("{}: {value:?} must start before it ends", at(.path))]
    RangeInverted { path: String, value: String },
    #[error("{}: ranges must be sorted by start", at(.path))]
    RangesUnsorted { path: String },
    #[error("{}: ranges overlap", at(.path))]
    RangesOverlap { path: String },
}

/// Something accepted but worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueWarning {
    /// A record carries a key the definition does not list.
    UnknownField { path: String, key: String },
}

/// `""` reads badly at the front of a message.
fn at(path: &str) -> &str {
    if path.is_empty() { "value" } else { path }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// Validate `value` against `def`. List min_items is not checked: it is a
/// completeness rule, not a shape rule.
pub fn validate_value(def: &QuestionDef, value: &Value) -> Result<Vec<ValueWarning>, ValueError> {
    if !def.is_addressable() {
        return Err(ValueError::NotAddressable);
    }
    let mut warnings = Vec::new();
    match def.answer_type {
        AnswerType::List => check_list(def, value, &mut warnings)?,
        AnswerType::Grid => check_grid(value)?,
        ty => check_scalar(ty, &def.options, value, "")?,
    }
    Ok(warnings)
}

fn mismatch(path: &str, expected: &'static str) -> ValueError {
    ValueError::TypeMismatch {
        path: path.to_owned(),
        expected,
    }
}

/// A string that is not blank.
fn non_empty<'a>(path: &str, value: &'a Value) -> Result<&'a str, ValueError> {
    let s = value.as_str().ok_or_else(|| mismatch(path, "string"))?;
    if s.trim().is_empty() {
        return Err(ValueError::Empty {
            path: path.to_owned(),
        });
    }
    Ok(s)
}

/// Scalar rules, shared by top-level answers and record fields.
fn check_scalar(
    ty: AnswerType,
    options: &[String],
    value: &Value,
    path: &str,
) -> Result<(), ValueError> {
    match ty {
        AnswerType::Bool => {
            value.as_bool().ok_or_else(|| mismatch(path, "boolean"))?;
        }
        AnswerType::Enum => {
            let s = value.as_str().ok_or_else(|| mismatch(path, "string"))?;
            if !options.iter().any(|o| o == s) {
                return Err(ValueError::BadOption {
                    path: path.to_owned(),
                    value: s.to_owned(),
                    valid: options.to_vec(),
                });
            }
        }
        AnswerType::Date => match value {
            Value::Datetime(_) => {
                return Err(ValueError::NativeDatetime {
                    path: path.to_owned(),
                });
            }
            Value::String(s) => {
                if !is_iso_date(s) {
                    return Err(ValueError::BadDate {
                        path: path.to_owned(),
                    });
                }
            }
            _ => return Err(mismatch(path, "date string")),
        },
        AnswerType::Number => match value {
            Value::Integer(_) => {}
            Value::Float(f) if f.is_finite() => {}
            Value::Float(_) => {
                return Err(ValueError::NotFinite {
                    path: path.to_owned(),
                });
            }
            _ => return Err(mismatch(path, "number")),
        },
        AnswerType::Text | AnswerType::File => {
            non_empty(path, value)?;
        }
        // Catalog validation keeps these out of record fields.
        AnswerType::List | AnswerType::Grid => {
            return Err(mismatch(path, "scalar"));
        }
    }
    Ok(())
}

/// Strict `YYYY-MM-DD` that is also a real calendar date.
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
        && s.parse::<jiff::civil::Date>().is_ok()
}

fn check_list(
    def: &QuestionDef,
    value: &Value,
    warnings: &mut Vec<ValueWarning>,
) -> Result<(), ValueError> {
    let items = value.as_array().ok_or_else(|| mismatch("", "array"))?;
    for (i, item) in items.iter().enumerate() {
        let path = format!("[{i}]");
        if def.fields.is_empty() {
            non_empty(&path, item)?;
        } else {
            check_record(&def.fields, item, &path, warnings)?;
        }
    }
    Ok(())
}

fn check_record(
    fields: &[FieldDef],
    item: &Value,
    path: &str,
    warnings: &mut Vec<ValueWarning>,
) -> Result<(), ValueError> {
    let table = item.as_table().ok_or_else(|| mismatch(path, "table"))?;
    for fd in fields {
        match table.get(&fd.name) {
            Some(v) => check_scalar(fd.answer_type, &fd.options, v, &join(path, &fd.name))?,
            None if fd.optional => {}
            None => {
                return Err(ValueError::MissingField {
                    path: path.to_owned(),
                    field: fd.name.clone(),
                });
            }
        }
    }
    let known: BTreeSet<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    for key in table.keys().filter(|k| !known.contains(k.as_str())) {
        warnings.push(ValueWarning::UnknownField {
            path: path.to_owned(),
            key: key.clone(),
        });
    }
    Ok(())
}

fn check_grid(value: &Value) -> Result<(), ValueError> {
    let table = value.as_table().ok_or_else(|| mismatch("", "table"))?;
    for (day, ranges) in table {
        if !DAYS.contains(&day.as_str()) {
            return Err(ValueError::BadDay {
                path: String::new(),
                key: day.clone(),
            });
        }
        let ranges = ranges.as_array().ok_or_else(|| mismatch(day, "array"))?;
        // (start, end) of the previous range, in minutes.
        let mut prev: Option<(u16, u16)> = None;
        for (i, r) in ranges.iter().enumerate() {
            let path = format!("{day}[{i}]");
            let s = r.as_str().ok_or_else(|| mismatch(&path, "string"))?;
            let bad = || ValueError::BadRange {
                path: path.clone(),
                value: s.to_owned(),
            };
            let (a, b) = s.split_once('-').ok_or_else(bad)?;
            let start = minutes(a, false).ok_or_else(bad)?;
            let end = minutes(b, true).ok_or_else(bad)?;
            if start >= end {
                return Err(ValueError::RangeInverted {
                    path,
                    value: s.to_owned(),
                });
            }
            if let Some((p_start, p_end)) = prev {
                if start < p_start {
                    return Err(ValueError::RangesUnsorted { path });
                }
                if start < p_end {
                    return Err(ValueError::RangesOverlap { path });
                }
            }
            prev = Some((start, end));
        }
    }
    Ok(())
}

/// `HH:MM` as minutes since midnight; `24:00` only when `is_end`.
fn minutes(s: &str, is_end: bool) -> Option<u16> {
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b':' || ![0, 1, 3, 4].iter().all(|&i| b[i].is_ascii_digit()) {
        return None;
    }
    let h: u16 = s[..2].parse().ok()?;
    let m: u16 = s[3..].parse().ok()?;
    if m > 59 || h > 24 || (h == 24 && !(is_end && m == 0)) {
        return None;
    }
    Some(h * 60 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A definition of the given type; `extra` is appended as more keys.
    fn def(ty: &str, extra: &str) -> QuestionDef {
        toml::from_str(&format!(
            "id='profile.x'\nsection='A'\nlabel='L'\nsensitivity='contact'\n\
             answer_type='{ty}'\n{extra}"
        ))
        .unwrap()
    }

    fn val(src: &str) -> Value {
        toml::from_str::<toml::Table>(&format!("v = {src}"))
            .unwrap()
            .remove("v")
            .unwrap()
    }

    fn ok(d: &QuestionDef, src: &str) -> Vec<ValueWarning> {
        validate_value(d, &val(src)).unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    fn err(d: &QuestionDef, src: &str) -> ValueError {
        validate_value(d, &val(src)).expect_err(src)
    }

    fn mismatch_at(path: &str, expected: &'static str) -> ValueError {
        mismatch(path, expected)
    }

    const EDU: &str = "fields = [\
        { name = 'school', label = 'S', answer_type = 'text' },\
        { name = 'start_date', label = 'D', answer_type = 'date' },\
        { name = 'level', label = 'V', answer_type = 'enum', options = ['ba', 'ma'], optional = true },\
        { name = 'gpa', label = 'G', answer_type = 'number', optional = true }]";

    #[test]
    fn bool_values() {
        let d = def("bool", "");
        ok(&d, "true");
        assert_eq!(err(&d, "'yes'"), mismatch_at("", "boolean"));
        assert_eq!(err(&d, "1"), mismatch_at("", "boolean"));
    }

    #[test]
    fn enum_values() {
        let d = def("enum", "options = ['a', 'b']");
        ok(&d, "'a'");
        assert!(matches!(err(&d, "'A'"), ValueError::BadOption { value, .. } if value == "A"));
        assert!(matches!(err(&d, "'c'"), ValueError::BadOption { .. }));
        assert_eq!(err(&d, "3"), mismatch_at("", "string"));
    }

    #[test]
    fn date_values() {
        let d = def("date", "");
        ok(&d, "'2026-02-28'");
        ok(&d, "'2024-02-29'");
        let bad = ValueError::BadDate {
            path: String::new(),
        };
        assert_eq!(err(&d, "'2026-02-30'"), bad);
        assert_eq!(err(&d, "'2026-13-01'"), bad);
        assert_eq!(err(&d, "'2026-2-3'"), bad);
        assert_eq!(err(&d, "'tomorrow'"), bad);
        assert_eq!(
            err(&d, "2026-02-03"),
            ValueError::NativeDatetime {
                path: String::new()
            }
        );
        assert_eq!(err(&d, "5"), mismatch_at("", "date string"));
    }

    #[test]
    fn number_values() {
        let d = def("number", "");
        ok(&d, "3");
        ok(&d, "-2.5");
        let nf = ValueError::NotFinite {
            path: String::new(),
        };
        assert_eq!(err(&d, "nan"), nf);
        assert_eq!(err(&d, "inf"), nf);
        assert_eq!(err(&d, "-inf"), nf);
        assert_eq!(err(&d, "'3'"), mismatch_at("", "number"));
    }

    #[test]
    fn text_and_file_values() {
        for ty in ["text", "file"] {
            let d = def(ty, "");
            ok(&d, "'hello'");
            let empty = ValueError::Empty {
                path: String::new(),
            };
            assert_eq!(err(&d, "''"), empty);
            assert_eq!(err(&d, "\"  \\t\""), empty);
            assert_eq!(err(&d, "4"), mismatch_at("", "string"));
        }
    }

    #[test]
    fn plain_list() {
        let d = def("list", "");
        assert!(ok(&d, "['a', 'b']").is_empty());
        ok(&d, "[]");
        assert_eq!(
            err(&d, "['a', ' ']"),
            ValueError::Empty { path: "[1]".into() }
        );
        assert_eq!(err(&d, "['a', 2]"), mismatch_at("[1]", "string"));
        assert_eq!(err(&d, "'a'"), mismatch_at("", "array"));
    }

    #[test]
    fn list_min_items_is_not_checked() {
        let d = def("list", "min_items = 2");
        ok(&d, "[]");
    }

    #[test]
    fn record_list() {
        let d = def("list", EDU);
        let good = "[{ school = 'MIT', start_date = '2020-09-01' }, \
                    { school = 'X', start_date = '2021-01-31', level = 'ma', gpa = 3.5 }]";
        assert!(ok(&d, good).is_empty());
        assert_eq!(
            err(&d, "[{ school = 'MIT' }]"),
            ValueError::MissingField {
                path: "[0]".into(),
                field: "start_date".into()
            }
        );
        assert_eq!(
            err(
                &d,
                "[{ school = 'MIT', start_date = '2020-09-01' }, { school = 1, start_date = '2020-09-01' }]"
            ),
            mismatch_at("[1].school", "string")
        );
        assert_eq!(
            err(&d, "[{ school = 'MIT', start_date = '2020-02-30' }]"),
            ValueError::BadDate {
                path: "[0].start_date".into()
            }
        );
        assert!(matches!(
            err(&d, "[{ school = 'M', start_date = '2020-09-01', level = 'phd' }]"),
            ValueError::BadOption { path, .. } if path == "[0].level"
        ));
        assert_eq!(err(&d, "['MIT']"), mismatch_at("[0]", "table"));
    }

    #[test]
    fn record_unknown_field_is_a_warning() {
        let d = def("list", EDU);
        let w = ok(
            &d,
            "[{ school = 'M', start_date = '2020-09-01' }, \
             { school = 'M', start_date = '2020-09-01', nick = 'x' }]",
        );
        assert_eq!(
            w,
            [ValueWarning::UnknownField {
                path: "[1]".into(),
                key: "nick".into()
            }]
        );
    }

    #[test]
    fn grid_values() {
        let d = def("grid", "");
        ok(&d, "{}");
        ok(&d, "{ mon = [], tue = ['09:00-12:00', '13:00-17:30'] }");
        // Touching ranges are fine; 24:00 is allowed as an end.
        ok(&d, "{ sun = ['00:00-12:00', '12:00-24:00'] }");
        let range = |p: &str, v: &str| ValueError::BadRange {
            path: p.into(),
            value: v.into(),
        };
        assert_eq!(
            err(&d, "{ mon = ['09:00-12:00', '11:00-13:00'] }"),
            ValueError::RangesOverlap {
                path: "mon[1]".into()
            }
        );
        assert_eq!(
            err(&d, "{ mon = ['13:00-14:00', '09:00-10:00'] }"),
            ValueError::RangesUnsorted {
                path: "mon[1]".into()
            }
        );
        assert_eq!(
            err(&d, "{ funday = [] }"),
            ValueError::BadDay {
                path: String::new(),
                key: "funday".into()
            }
        );
        assert_eq!(
            err(&d, "{ mon = ['24:00-24:00'] }"),
            range("mon[0]", "24:00-24:00")
        );
        assert_eq!(
            err(&d, "{ mon = ['9:00-10:00'] }"),
            range("mon[0]", "9:00-10:00")
        );
        assert_eq!(err(&d, "{ mon = ['09:00'] }"), range("mon[0]", "09:00"));
        assert_eq!(
            err(&d, "{ mon = ['09:60-10:00'] }"),
            range("mon[0]", "09:60-10:00")
        );
        assert_eq!(
            err(&d, "{ mon = ['09:00-24:01'] }"),
            range("mon[0]", "09:00-24:01")
        );
        assert_eq!(
            err(&d, "{ mon = ['09:00-25:00'] }"),
            range("mon[0]", "09:00-25:00")
        );
        assert!(matches!(
            err(&d, "{ mon = ['10:00-10:00'] }"),
            ValueError::RangeInverted { .. }
        ));
        assert!(matches!(
            err(&d, "{ mon = ['11:00-10:00'] }"),
            ValueError::RangeInverted { .. }
        ));
        assert_eq!(
            err(&d, "{ mon = '09:00-10:00' }"),
            mismatch_at("mon", "array")
        );
        assert_eq!(err(&d, "{ mon = [9] }"), mismatch_at("mon[0]", "string"));
        assert_eq!(err(&d, "['09:00-10:00']"), mismatch_at("", "table"));
    }

    #[test]
    fn not_addressable() {
        for sens in ["attestation", "prohibited"] {
            let mut d = def("bool", "");
            d.sensitivity = toml::Value::String(sens.into()).try_into().unwrap();
            assert_eq!(err(&d, "true"), ValueError::NotAddressable);
        }
    }

    #[test]
    fn messages_are_short_and_specific() {
        let d = def("list", EDU);
        assert_eq!(
            err(&d, "[{ school = 'M', start_date = '2020-02-30' }]").to_string(),
            "[0].start_date: not a valid YYYY-MM-DD date"
        );
        assert_eq!(
            err(&def("bool", ""), "1").to_string(),
            "value: expected boolean"
        );
    }
}
