//! Derived suggestions (m2a question-bank design §5): a verified answer can
//! suggest values for other questions through their `derive` rules.
//!
//! Pure function over data the caller loaded. Only verified answers are
//! sources and derived values are never sources, so there are no chains.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use toml::Value;

use super::Catalog;

/// One suggested value and the question it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Derived {
    pub id: String,
    pub value: Value,
    pub from: String,
}

impl Derived {
    /// Provenance tag stored with the suggestion: `derived:<from>`.
    pub fn source(&self) -> String {
        format!("derived:{}", self.from)
    }
}

/// Integers and floats compare numerically; everything else exactly.
pub(crate) fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Integer(i), Value::Float(f)) | (Value::Float(f), Value::Integer(i)) => {
            (*i as f64) == *f
        }
        _ => a == b,
    }
}

/// Suggestions for every question that has none yet. `verified` holds the
/// user's verified answer values by id; `occupied` the ids that already have
/// any stored value or an explicit policy. Pack questions apply only for
/// `occupations`, as in coverage. The first matching rule per question wins;
/// output is in id order.
pub fn derive(
    catalog: &Catalog,
    occupations: &[String],
    verified: &BTreeMap<String, Value>,
    occupied: &BTreeSet<String>,
) -> Vec<Derived> {
    let mut out = Vec::new();
    for def in catalog.iter() {
        if def.derive.is_empty() || occupied.contains(&def.id) {
            continue;
        }
        if catalog
            .pack_of(&def.id)
            .is_some_and(|p| !occupations.iter().any(|o| o == p))
        {
            continue;
        }
        let hit = def
            .derive
            .iter()
            .find(|r| verified.get(&r.from).is_some_and(|v| same(v, &r.equals)));
        if let Some(rule) = hit {
            out.push(Derived {
                id: def.id.clone(),
                value: rule.value.clone(),
                from: rule.from.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(id: &str, section: &str, sens: &str, ty: &str, extra: &str) -> String {
        format!(
            "[[question]]\nid = '{id}'\nsection = '{section}'\nlabel = 'L'\n\
             sensitivity = '{sens}'\nanswer_type = '{ty}'\n{extra}\n"
        )
    }

    fn cat(files: &[(&str, &str)]) -> Catalog {
        Catalog::parse(files).unwrap_or_else(|e| panic!("{e}"))
    }

    fn verified(items: &[(&str, Value)]) -> BTreeMap<String, Value> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    /// `legal.a` bool source; `legal.b` bool target with the given rules.
    fn pair(rules: &str) -> Catalog {
        let body = format!(
            "{}{}",
            q("legal.a", "C", "legal", "bool", ""),
            q(
                "legal.b",
                "C",
                "legal",
                "bool",
                &format!("derive = [{rules}]")
            )
        );
        cat(&[("legal.toml", &body)])
    }

    #[test]
    fn source_tag() {
        let d = Derived {
            id: "legal.b".into(),
            value: Value::Boolean(true),
            from: "legal.a".into(),
        };
        assert_eq!(d.source(), "derived:legal.a");
    }

    #[test]
    fn first_rule_wins() {
        let c = pair(
            "{ from = 'legal.a', equals = true, value = true }, \
             { from = 'legal.a', equals = false, value = false }",
        );
        let got = derive(
            &c,
            &[],
            &verified(&[("legal.a", Value::Boolean(false))]),
            &none(),
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].value, Value::Boolean(false));

        // Two matching rules from different sources: file order decides.
        let body = format!(
            "{}{}{}",
            q("legal.a", "C", "legal", "bool", ""),
            q("legal.c", "C", "legal", "bool", ""),
            q(
                "legal.b",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.c', equals = true, value = false }, \
                 { from = 'legal.a', equals = true, value = true }]"
            )
        );
        let c = cat(&[("legal.toml", &body)]);
        let v = verified(&[
            ("legal.a", Value::Boolean(true)),
            ("legal.c", Value::Boolean(true)),
        ]);
        let got = derive(&c, &[], &v, &none());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].from, "legal.c");
        assert_eq!(got[0].value, Value::Boolean(false));
    }

    #[test]
    fn unverified_source_is_ignored() {
        let c = pair("{ from = 'legal.a', equals = true, value = true }");
        assert!(derive(&c, &[], &BTreeMap::new(), &none()).is_empty());
        let v = verified(&[("legal.a", Value::Boolean(false))]);
        assert!(derive(&c, &[], &v, &none()).is_empty());
    }

    #[test]
    fn occupied_target_is_skipped() {
        let c = pair("{ from = 'legal.a', equals = true, value = true }");
        let v = verified(&[("legal.a", Value::Boolean(true))]);
        let occupied: BTreeSet<String> = ["legal.b".to_owned()].into();
        assert!(derive(&c, &[], &v, &occupied).is_empty());
        assert_eq!(derive(&c, &[], &v, &none()).len(), 1);
    }

    #[test]
    fn numbers_compare_numerically() {
        let body = format!(
            "{}{}",
            q("legal.n", "C", "numeric", "number", ""),
            q(
                "legal.b",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.n', equals = 3, value = true }]"
            )
        );
        let c = cat(&[("legal.toml", &body)]);
        for v in [Value::Float(3.0), Value::Integer(3)] {
            let got = derive(&c, &[], &verified(&[("legal.n", v)]), &none());
            assert_eq!(got.len(), 1);
        }
        let got = derive(
            &c,
            &[],
            &verified(&[("legal.n", Value::Float(3.5))]),
            &none(),
        );
        assert!(got.is_empty());
    }

    #[test]
    fn enum_strings_compare_exactly() {
        let body = format!(
            "{}{}",
            q(
                "legal.e",
                "C",
                "legal",
                "enum",
                "options = ['yes', 'no']\n\
                 options_normalization = { yes = ['Yes'], no = ['No'] }"
            ),
            q(
                "legal.b",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.e', equals = 'yes', value = true }]"
            )
        );
        let c = cat(&[("legal.toml", &body)]);
        let hit = verified(&[("legal.e", Value::String("yes".into()))]);
        assert_eq!(derive(&c, &[], &hit, &none()).len(), 1);
        let miss = verified(&[("legal.e", Value::String("no".into()))]);
        assert!(derive(&c, &[], &miss, &none()).is_empty());
    }

    #[test]
    fn pack_questions_follow_occupations() {
        let core = q("legal.a", "C", "legal", "bool", "");
        let pack = format!(
            "pack = 'trades'\nreview_status = 'unreviewed'\nreviewed_by = []\n{}",
            q(
                "legal.trades.b",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.a', equals = true, value = true }]"
            )
        );
        let c = cat(&[("legal.toml", &core), ("packs/trades.toml", &pack)]);
        let v = verified(&[("legal.a", Value::Boolean(true))]);
        assert!(derive(&c, &[], &v, &none()).is_empty());
        assert!(derive(&c, &["other".to_owned()], &v, &none()).is_empty());
        let got = derive(&c, &["trades".to_owned()], &v, &none());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "legal.trades.b");
    }

    #[test]
    fn derived_values_do_not_chain() {
        let body = format!(
            "{}{}{}",
            q("legal.a", "C", "legal", "bool", ""),
            q(
                "legal.b",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.a', equals = true, value = true }]"
            ),
            q(
                "legal.c",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.b', equals = true, value = true }]"
            )
        );
        let c = cat(&[("legal.toml", &body)]);
        let v = verified(&[("legal.a", Value::Boolean(true))]);
        let got = derive(&c, &[], &v, &none());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "legal.b");
    }

    #[test]
    fn output_is_sorted_by_id() {
        // Declared z-first; output must still be id order.
        let body = format!(
            "{}{}{}",
            q("legal.a", "C", "legal", "bool", ""),
            q(
                "legal.z",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.a', equals = true, value = true }]"
            ),
            q(
                "legal.m",
                "C",
                "legal",
                "bool",
                "derive = [{ from = 'legal.a', equals = true, value = true }]"
            )
        );
        let c = cat(&[("legal.toml", &body)]);
        let v = verified(&[("legal.a", Value::Boolean(true))]);
        let ids: Vec<String> = derive(&c, &[], &v, &none())
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, ["legal.m", "legal.z"]);
    }

    #[test]
    fn us_citizen_example() {
        let from = "derive = [{ from = 'legal.authorization.us_citizen', equals = true, value = ";
        let body = format!(
            "{}{}{}{}",
            q("legal.authorization.us_citizen", "C", "legal", "bool", ""),
            q(
                "legal.authorization.require_sponsorship_now",
                "C",
                "legal",
                "bool",
                &format!("{from}false }}]")
            ),
            q(
                "legal.authorization.require_sponsorship_future",
                "C",
                "legal",
                "bool",
                &format!("{from}false }}]")
            ),
            q(
                "legal.authorization.us_person",
                "C",
                "legal",
                "bool",
                &format!("{from}true }}]")
            )
        );
        let c = cat(&[("legal.toml", &body)]);
        let v = verified(&[("legal.authorization.us_citizen", Value::Boolean(true))]);
        let got = derive(&c, &[], &v, &none());
        let pairs: Vec<(&str, &Value)> = got.iter().map(|d| (d.id.as_str(), &d.value)).collect();
        assert_eq!(
            pairs,
            [
                (
                    "legal.authorization.require_sponsorship_future",
                    &Value::Boolean(false)
                ),
                (
                    "legal.authorization.require_sponsorship_now",
                    &Value::Boolean(false)
                ),
                ("legal.authorization.us_person", &Value::Boolean(true)),
            ]
        );
        assert!(
            got.iter()
                .all(|d| d.source() == "derived:legal.authorization.us_citizen")
        );
    }
}
