//! Reading bank documents against the catalog (design §1, §3).
//!
//! A question id is `<file>.<toml path>`; these functions look at an
//! already-parsed bank file and perform no I/O.

use super::{AnswerType, Catalog, QuestionDef, Stored};

/// What `doc` (the parsed file named by the first segment of `def.id`)
/// stores for `def`, without the value itself. Anything absent or
/// malformed is `None`.
pub fn stored_at(doc: &toml::Table, def: &QuestionDef) -> Option<Stored> {
    let mut segments = def.id.split('.').skip(1).peekable();
    let mut table = doc;
    let mut leaf = None;
    while let Some(seg) = segments.next() {
        let value = table.get(seg)?;
        if segments.peek().is_none() {
            leaf = Some(value);
        } else {
            table = value.as_table()?;
        }
    }
    let leaf = leaf?;
    if def.answer_type == AnswerType::List {
        let rows = leaf.as_array()?;
        let mut verified = 0;
        for row in rows {
            if row.as_table()?.get("verified")?.as_bool()? {
                verified += 1;
            }
        }
        Some(Stored::List {
            items: rows.len(),
            verified,
        })
    } else {
        let verified = leaf.as_table()?.get("verified")?.as_bool()?;
        Some(Stored::Scalar { verified })
    }
}

/// Dotted ids of the stored items in `doc` (the parsed bank file named
/// `file`, e.g. `"legal"`) that no question in `catalog` defines. An item
/// is a table with a `verified` key or an array of tables; plain keys such
/// as `eeo.autofill` and tables that only lead to items are not reported.
/// Such values are kept, never deleted (design §3).
pub fn unknown_paths(catalog: &Catalog, file: &str, doc: &toml::Table) -> Vec<String> {
    let mut out = Vec::new();
    walk(catalog, file, doc, &mut out);
    out
}

fn walk(catalog: &Catalog, prefix: &str, table: &toml::Table, out: &mut Vec<String>) {
    for (key, value) in table {
        let id = format!("{prefix}.{key}");
        let is_item = match value {
            toml::Value::Table(t) if !t.contains_key("verified") => {
                walk(catalog, &id, t, out);
                false
            }
            toml::Value::Table(_) => true,
            toml::Value::Array(rows) => !rows.is_empty() && rows.iter().all(toml::Value::is_table),
            _ => false,
        };
        if is_item && catalog.get(&id).is_none() {
            out.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, answer_type: &str) -> QuestionDef {
        toml::from_str(&format!(
            "id='{id}'\nsection='C'\nlabel='x'\nsensitivity='legal'\nanswer_type='{answer_type}'"
        ))
        .unwrap()
    }

    fn doc(text: &str) -> toml::Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn scalar_reads_verified_flag() {
        let d = doc("[authorization.us_citizen]\nvalue = true\nverified = false\n");
        let q = def("legal.authorization.us_citizen", "bool");
        assert_eq!(stored_at(&d, &q), Some(Stored::Scalar { verified: false }));
        let d = doc("[authorization.us_citizen]\nverified = true\n");
        assert_eq!(stored_at(&d, &q), Some(Stored::Scalar { verified: true }));
    }

    #[test]
    fn scalar_absent_or_malformed_is_none() {
        let q = def("legal.authorization.us_citizen", "bool");
        assert_eq!(stored_at(&doc(""), &q), None);
        assert_eq!(stored_at(&doc("authorization = 3"), &q), None);
        assert_eq!(
            stored_at(&doc("[authorization]\nus_citizen = true"), &q),
            None
        );
        assert_eq!(
            stored_at(&doc("[authorization.us_citizen]\nverified = \"yes\""), &q),
            None
        );
        assert_eq!(
            stored_at(&doc("[authorization.us_citizen]\nvalue = true"), &q),
            None
        );
    }

    #[test]
    fn list_counts_verified_entries() {
        let q = def("experience.licenses", "list");
        let d = doc("[[licenses]]\nverified = true\n[[licenses]]\nverified = false\n");
        assert_eq!(
            stored_at(&d, &q),
            Some(Stored::List {
                items: 2,
                verified: 1
            })
        );
        assert_eq!(
            stored_at(&doc("licenses = []"), &q),
            Some(Stored::List {
                items: 0,
                verified: 0
            })
        );
        assert_eq!(stored_at(&doc("licenses = [1]"), &q), None);
        assert_eq!(stored_at(&doc("[licenses]\nverified = true"), &q), None);
        assert_eq!(stored_at(&doc(""), &q), None);
    }

    fn catalog() -> Catalog {
        let text = "[[question]]\nid='legal.authorization.us_citizen'\nsection='C'\nlabel='x'\n\
                    sensitivity='legal'\nanswer_type='bool'\n\
                    [[question]]\nid='legal.licenses'\nsection='C'\nlabel='x'\n\
                    sensitivity='legal'\nanswer_type='list'\n";
        Catalog::parse(&[("legal.toml", text)]).unwrap()
    }

    #[test]
    fn unknown_paths_reports_undefined_items_only() {
        let d = doc("autofill = true\n\
             [authorization.us_citizen]\nverified = true\n\
             [authorization.mystery]\nverified = false\n\
             [[licenses]]\nverified = true\n\
             [[stray]]\nverified = true\n\
             [deep.er.item]\nverified = true\n");
        assert_eq!(
            unknown_paths(&catalog(), "legal", &d),
            vec![
                "legal.authorization.mystery",
                "legal.deep.er.item",
                "legal.stray"
            ]
        );
    }

    #[test]
    fn unknown_paths_ignores_plain_keys_and_prefix_tables() {
        let d = doc("autofill = true\nnote = [1, 2]\n[authorization]\nx = 1\n");
        assert!(unknown_paths(&catalog(), "legal", &d).is_empty());
    }
}
