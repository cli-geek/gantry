//! Data shipped inside the binary: list-feed definitions (`data/feeds/`)
//! and per-occupation seed company lists (`data/companies/`).

use std::path::Path;

use gantry_store::{CompanyEntry, CompanyList, StoreError};
use serde::Deserialize;

const FEEDS: &[(&str, &str)] = &[
    (
        "simplify-new-grad.toml",
        include_str!("../../../data/feeds/simplify-new-grad.toml"),
    ),
    (
        "simplify-internships.toml",
        include_str!("../../../data/feeds/simplify-internships.toml"),
    ),
];

const COMPANY_LISTS: &[(&str, &str)] = &[(
    "technology.toml",
    include_str!("../../../data/companies/technology.toml"),
)];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedFormat {
    SimplifyListings,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedDef {
    pub id: String,
    pub name: String,
    pub format: FeedFormat,
    pub url: String,
    pub homepage: Option<String>,
    /// Occupation packs whose users the feed serves; shown when choosing
    /// feeds, never used to enable one.
    pub occupations: Vec<String>,
}

impl FeedDef {
    /// The `posting_sources.source` and external `board_token` value.
    pub fn source(&self) -> String {
        format!("feed:{}", self.id)
    }
}

pub fn feeds() -> Result<Vec<FeedDef>, StoreError> {
    FEEDS
        .iter()
        .map(|(file, text)| {
            toml::from_str(text).map_err(|e| StoreError::Toml {
                path: Path::new("data/feeds").join(file),
                message: e.to_string(),
            })
        })
        .collect()
}

/// Seed companies for the given occupation pack ids.
pub fn seed_companies(occupations: &[String]) -> Result<Vec<CompanyEntry>, StoreError> {
    let mut out = Vec::new();
    for (file, text) in COMPANY_LISTS {
        let list = CompanyList::parse(text, &Path::new("data/companies").join(file))?;
        if list
            .occupation
            .as_ref()
            .is_some_and(|o| occupations.contains(o))
        {
            out.extend(list.companies);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_data_parses() {
        let feeds = feeds().unwrap();
        assert_eq!(feeds.len(), FEEDS.len());
        assert!(feeds.iter().all(|f| f.url.starts_with("https://")));
        assert!(!seed_companies(&["technology".into()]).unwrap().is_empty());
    }

    #[test]
    fn seed_follows_occupations_only() {
        assert!(seed_companies(&[]).unwrap().is_empty());
        assert!(
            seed_companies(&["dental hygiene".into()])
                .unwrap()
                .is_empty()
        );
    }
}
