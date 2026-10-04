//! TOML files in the config directory (§9.1).
//!
//! The headless run reads one snapshot at start and never writes TOML.
//! Interactive commands write through [`save_toml`], which replaces the
//! file atomically.

use std::path::{Path, PathBuf};

use gantry_core::Ats;
use gantry_core::bank::{Banks, ExperienceBank, LegalBank, ProfileBank};
use gantry_core::search::SearchProfile;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::StoreError;

/// `settings.toml`. Defaults enable nothing occupation-specific.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub discovery: DiscoverySettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiscoverySettings {
    /// Ids of the list feeds in `data/feeds/` to read.
    pub feeds: Vec<String>,
    /// Read the monthly Hacker News "Who is hiring?" thread.
    pub hacker_news: bool,
    /// Look up company names from every source on the board APIs.
    pub slug_probing: bool,
}

impl Default for DiscoverySettings {
    fn default() -> Self {
        Self {
            feeds: Vec::new(),
            hacker_news: false,
            slug_probing: true,
        }
    }
}

/// One company → board mapping, in `data/companies/*.toml` or the user's
/// `companies.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanyEntry {
    pub name: String,
    pub ats: Ats,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staffing_agency: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CompanyList {
    /// Occupation pack this bundled list belongs to; absent in the user's
    /// own file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occupation: Option<String>,
    #[serde(rename = "company")]
    pub companies: Vec<CompanyEntry>,
}

impl CompanyList {
    pub fn parse(text: &str, origin: &Path) -> Result<Self, StoreError> {
        let list: Self = toml::from_str(text).map_err(|e| toml_error(origin, &e))?;
        if let Some(bad) = list.companies.iter().find(|c| c.ats == Ats::External) {
            return Err(StoreError::Toml {
                path: origin.to_path_buf(),
                message: format!(
                    "company \"{}\": ats must be a board API (greenhouse, lever, lever_eu, ashby)",
                    bad.name
                ),
            });
        }
        Ok(list)
    }
}

/// Everything a run reads from the config directory, read once.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigSnapshot {
    pub search: SearchProfile,
    pub settings: Settings,
    pub banks: Banks,
    /// The user's own `companies.toml`.
    pub companies: Vec<CompanyEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFile {
    Search,
    Settings,
    Profile,
    Legal,
    Experience,
    Companies,
}

impl ConfigFile {
    pub const ALL: [Self; 6] = [
        Self::Search,
        Self::Settings,
        Self::Profile,
        Self::Legal,
        Self::Experience,
        Self::Companies,
    ];

    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Search => "search.toml",
            Self::Settings => "settings.toml",
            Self::Profile => "profile.toml",
            Self::Legal => "legal.toml",
            Self::Experience => "experience.toml",
            Self::Companies => "companies.toml",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    Missing,
    Valid,
    Invalid(String),
}

fn toml_error(path: &Path, e: &toml::de::Error) -> StoreError {
    StoreError::Toml {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

fn read_optional(path: &Path) -> Result<Option<String>, StoreError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StoreError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// A missing file reads as the type's default: an empty profile.
fn read_toml<T: DeserializeOwned + Default>(path: &Path) -> Result<T, StoreError> {
    match read_optional(path)? {
        Some(text) => toml::from_str(&text).map_err(|e| toml_error(path, &e)),
        None => Ok(T::default()),
    }
}

pub fn load_snapshot(config_dir: &Path) -> Result<ConfigSnapshot, StoreError> {
    let path = |f: ConfigFile| config_dir.join(f.file_name());
    let companies_path = path(ConfigFile::Companies);
    let companies = match read_optional(&companies_path)? {
        Some(text) => CompanyList::parse(&text, &companies_path)?.companies,
        None => Vec::new(),
    };
    Ok(ConfigSnapshot {
        search: read_toml::<SearchProfile>(&path(ConfigFile::Search))?,
        settings: read_toml::<Settings>(&path(ConfigFile::Settings))?,
        banks: Banks {
            profile: read_toml::<ProfileBank>(&path(ConfigFile::Profile))?,
            legal: read_toml::<LegalBank>(&path(ConfigFile::Legal))?,
            experience: read_toml::<ExperienceBank>(&path(ConfigFile::Experience))?,
        },
        companies,
    })
}

/// Parses each config file on its own so `doctor` can report every
/// problem at once instead of stopping at the first.
pub fn check_files(config_dir: &Path) -> Vec<(PathBuf, FileState)> {
    ConfigFile::ALL
        .iter()
        .map(|f| {
            let path = config_dir.join(f.file_name());
            let state = match read_optional(&path) {
                Ok(None) => FileState::Missing,
                Err(e) => FileState::Invalid(e.to_string()),
                Ok(Some(text)) => {
                    let parsed = match f {
                        ConfigFile::Search => parses::<SearchProfile>(&text),
                        ConfigFile::Settings => parses::<Settings>(&text),
                        ConfigFile::Profile => parses::<ProfileBank>(&text),
                        ConfigFile::Legal => parses::<LegalBank>(&text),
                        ConfigFile::Experience => parses::<ExperienceBank>(&text),
                        ConfigFile::Companies => CompanyList::parse(&text, &path)
                            .map(drop)
                            .map_err(|e| e.to_string()),
                    };
                    match parsed {
                        Ok(()) => FileState::Valid,
                        Err(message) => FileState::Invalid(message),
                    }
                }
            };
            (path, state)
        })
        .collect()
}

fn parses<T: DeserializeOwned>(text: &str) -> Result<(), String> {
    toml::from_str::<T>(text)
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Serializes `value` and atomically replaces `path` with it (§9.1).
pub fn save_toml<T: Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    let text = toml::to_string_pretty(value).map_err(|e| StoreError::Toml {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    gantry_platform::write_atomic(path, text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_reads_as_empty_profile() {
        let dir = tempfile::tempdir().unwrap();
        let snap = load_snapshot(&dir.path().join("absent")).unwrap();
        assert_eq!(snap, ConfigSnapshot::default());
        assert!(snap.settings.discovery.slug_probing);
        assert!(!snap.settings.discovery.hacker_news);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            discovery: DiscoverySettings {
                feeds: vec!["simplify-new-grad".into()],
                hacker_news: true,
                slug_probing: false,
            },
        };
        save_toml(&dir.path().join("settings.toml"), &settings).unwrap();
        assert_eq!(load_snapshot(dir.path()).unwrap().settings, settings);
    }

    #[test]
    fn check_files_reports_each_problem() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("search.toml"), "[titles]\nbogus = 1\n").unwrap();
        std::fs::write(
            dir.path().join("companies.toml"),
            "[[company]]\nname = \"X\"\nats = \"external\"\ntoken = \"x\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("settings.toml"), "").unwrap();
        let states: Vec<_> = check_files(dir.path())
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        assert!(matches!(states[0], FileState::Invalid(ref m) if m.contains("bogus")));
        assert_eq!(states[1], FileState::Valid);
        assert_eq!(states[2], FileState::Missing);
        assert!(matches!(states[5], FileState::Invalid(ref m) if m.contains("board API")));
        assert!(load_snapshot(dir.path()).is_err());
    }
}
