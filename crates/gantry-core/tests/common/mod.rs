//! Loads the bundled question catalog from `data/questions/` for tests.
//! Files whose name starts with `_` (the pack template) are skipped.

use std::fs;
use std::path::{Path, PathBuf};

use gantry_core::questions::Catalog;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/questions")
}

/// `(path relative to data/questions, contents)`, sorted by path.
pub fn bundled_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    collect(&root(), "", &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect(&path, &format!("{prefix}{name}/"), out);
        } else if name.ends_with(".toml") && !name.starts_with('_') {
            out.push((
                format!("{prefix}{name}"),
                fs::read_to_string(&path).unwrap(),
            ));
        }
    }
}

/// The bundled catalog; panics with every validation error.
pub fn bundled() -> Catalog {
    let files = bundled_files();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    Catalog::parse(&refs).unwrap_or_else(|e| panic!("bundled catalog:\n{e}"))
}
