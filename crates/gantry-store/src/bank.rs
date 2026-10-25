//! The question bank's TOML documents (design §1, §3, §5).
//!
//! Bank files are read as plain tables so values this version does not
//! know are kept and written back untouched.

use std::collections::BTreeMap;
use std::path::Path;

use gantry_core::bank::Item;
use gantry_core::questions::{AnswerState, Catalog, PolicyMode, state_of, stored_at};

use crate::StoreError;
use crate::config::{ConfigFile, read_optional, toml_error};

/// The bank files that hold answers, each as a table keyed by file stem.
/// `answers.toml` is read into [`BankDocs::policy`] instead.
const ANSWER_FILES: [ConfigFile; 8] = [
    ConfigFile::Profile,
    ConfigFile::Legal,
    ConfigFile::Experience,
    ConfigFile::Eeo,
    ConfigFile::Availability,
    ConfigFile::Stories,
    ConfigFile::Approach,
    ConfigFile::Motivation,
];

/// The user's bank files, parsed but not interpreted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BankDocs {
    /// Parsed file per stem (`"legal"` for `legal.toml`); a missing file is
    /// an empty table.
    pub files: BTreeMap<String, toml::Table>,
    /// Standing instruction per question id, from `answers.toml`.
    pub policy: BTreeMap<String, PolicyMode>,
}

impl BankDocs {
    /// The state of every addressable question, ready for
    /// [`gantry_core::questions::coverage`].
    pub fn states(&self, catalog: &Catalog) -> BTreeMap<String, AnswerState> {
        catalog
            .iter()
            .filter(|def| def.is_addressable())
            .map(|def| {
                let stored = def
                    .id
                    .split('.')
                    .next()
                    .and_then(|stem| self.files.get(stem))
                    .and_then(|doc| stored_at(doc, def));
                let policy = self.policy.get(&def.id).copied();
                (def.id.clone(), state_of(def, stored, policy))
            })
            .collect()
    }
}

/// Reads a file as a table; a missing file is an empty table.
fn read_table(path: &Path) -> Result<toml::Table, StoreError> {
    match read_optional(path)? {
        Some(text) => text.parse().map_err(|e| toml_error(path, &e)),
        None => Ok(toml::Table::new()),
    }
}

/// Reads every bank file in `config_dir`. An unparsable file is a
/// [`StoreError::Toml`].
pub fn load_bank_docs(config_dir: &Path) -> Result<BankDocs, StoreError> {
    let mut docs = BankDocs::default();
    for file in ANSWER_FILES {
        let name = file.file_name();
        let table = read_table(&config_dir.join(name))?;
        docs.files
            .insert(name.trim_end_matches(".toml").to_owned(), table);
    }
    let path = config_dir.join(ConfigFile::Answers.file_name());
    let answers = read_table(&path)?;
    if let Some(policy) = answers.get("policy") {
        let rows = policy.as_table().ok_or_else(|| StoreError::Toml {
            path: path.clone(),
            message: "policy must be a table of items".to_owned(),
        })?;
        for (id, row) in rows {
            let item: Item<PolicyMode> = row.clone().try_into().map_err(|e| StoreError::Toml {
                path: path.clone(),
                message: format!("policy.\"{id}\": {e}"),
            })?;
            docs.policy.insert(id.clone(), item.value);
        }
    }
    Ok(docs)
}

/// Reads the table at `path` (a missing file is empty), applies `f`, and
/// atomically writes the result back. An unparsable file is an error and
/// is left untouched. Keys `f` does not touch survive.
///
/// There is no lock: with concurrent writers the last one wins (design §5).
/// Only interactive commands may call this; `gantry run` never writes TOML.
pub fn update_table<R>(
    path: &Path,
    f: impl FnOnce(&mut toml::Table) -> R,
) -> Result<R, StoreError> {
    let mut table = read_table(path)?;
    let result = f(&mut table);
    let text = toml::to_string_pretty(&table).map_err(|e| StoreError::Toml {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    gantry_platform::write_atomic(path, text.as_bytes())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legal.toml");
        let n = update_table(&path, |t| {
            t.insert("a".into(), toml::Value::Integer(1));
            7
        })
        .unwrap();
        assert_eq!(n, 7);
        assert_eq!(read_table(&path).unwrap()["a"], toml::Value::Integer(1));
    }

    #[test]
    fn update_keeps_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legal.toml");
        std::fs::write(&path, "[future.thing]\nverified = true\nextra = 'x'\n").unwrap();
        update_table(&path, |t| {
            t.insert("a".into(), toml::Value::Boolean(true));
        })
        .unwrap();
        let t = read_table(&path).unwrap();
        assert_eq!(t["future"]["thing"]["extra"].as_str(), Some("x"));
        assert_eq!(t["a"].as_bool(), Some(true));
    }

    #[test]
    fn update_refuses_an_unparsable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legal.toml");
        std::fs::write(&path, "not [ toml").unwrap();
        let err = update_table(&path, |_| ()).unwrap_err();
        assert!(matches!(err, StoreError::Toml { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not [ toml");
    }

    #[test]
    fn load_reads_policy_and_treats_missing_files_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("answers.toml"),
            "[policy.\"eeo.gender\"]\nid = \"p\"\nvalue = \"decline\"\nverified = true\n\
             created_at = \"2026-09-01T00:00:00Z\"\nupdated_at = \"2026-09-01T00:00:00Z\"\n",
        )
        .unwrap();
        let docs = load_bank_docs(dir.path()).unwrap();
        assert_eq!(docs.policy["eeo.gender"], PolicyMode::Decline);
        assert_eq!(docs.files.len(), 8);
        assert!(docs.files["legal"].is_empty());
    }

    #[test]
    fn load_rejects_a_bad_policy_row_and_a_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("answers.toml"),
            "[policy.\"x.y\"]\nvalue = \"sometimes\"\n",
        )
        .unwrap();
        assert!(matches!(
            load_bank_docs(dir.path()),
            Err(StoreError::Toml { .. })
        ));
        std::fs::remove_file(dir.path().join("answers.toml")).unwrap();
        std::fs::write(dir.path().join("eeo.toml"), "= nope").unwrap();
        assert!(matches!(
            load_bank_docs(dir.path()),
            Err(StoreError::Toml { .. })
        ));
    }
}
