//! The question catalog shipped inside the binary (`data/questions/`).

use gantry_core::questions::{Catalog, CatalogErrors};

/// Every catalog file except the `_`-prefixed pack template, by path
/// relative to `data/questions/`.
const FILES: &[(&str, &str)] = &[
    (
        "approach.toml",
        include_str!("../../../data/questions/approach.toml"),
    ),
    (
        "availability.toml",
        include_str!("../../../data/questions/availability.toml"),
    ),
    ("eeo.toml", include_str!("../../../data/questions/eeo.toml")),
    (
        "experience.toml",
        include_str!("../../../data/questions/experience.toml"),
    ),
    (
        "legal.toml",
        include_str!("../../../data/questions/legal.toml"),
    ),
    (
        "motivation.toml",
        include_str!("../../../data/questions/motivation.toml"),
    ),
    (
        "profile.toml",
        include_str!("../../../data/questions/profile.toml"),
    ),
    (
        "stories.toml",
        include_str!("../../../data/questions/stories.toml"),
    ),
    (
        "packs/technology.toml",
        include_str!("../../../data/questions/packs/technology.toml"),
    ),
];

/// The bundled catalog. Failure means the shipped data is broken.
pub fn bundled_catalog() -> Result<Catalog, CatalogErrors> {
    Catalog::parse(FILES)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;

    #[test]
    fn bundled_catalog_parses() {
        let catalog = bundled_catalog().unwrap();
        assert!(!catalog.is_empty());
    }

    fn on_disk(dir: &Path, prefix: &str, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            if entry.path().is_dir() {
                on_disk(&entry.path(), &format!("{prefix}{name}/"), out);
            } else if name.ends_with(".toml") && !name.starts_with('_') {
                out.insert(format!("{prefix}{name}"));
            }
        }
    }

    #[test]
    fn file_list_matches_the_data_directory() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/questions");
        let mut disk = BTreeSet::new();
        on_disk(&root, "", &mut disk);
        let listed: BTreeSet<String> = FILES.iter().map(|(p, _)| (*p).to_owned()).collect();
        assert_eq!(listed, disk);
    }
}
