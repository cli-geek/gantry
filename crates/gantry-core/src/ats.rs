use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where a posting lives. `External` covers every site without a board API
/// adapter; such postings are still discovered and filtered (§4.1.1).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Ats {
    Greenhouse,
    Lever,
    /// Lever's EU data region: same product, separate API host.
    LeverEu,
    Ashby,
    External,
}

impl Ats {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::LeverEu => "lever_eu",
            Self::Ashby => "ashby",
            Self::External => "external",
        }
    }

    /// The board token as stored. Greenhouse and Ashby answer for any
    /// letter case, so one board reached through differently cased URLs is
    /// stored once; Lever tokens are case-sensitive (checked live
    /// 2026-10-04).
    pub fn canonical_token(self, token: &str) -> String {
        match self {
            Self::Greenhouse | Self::Ashby => token.to_lowercase(),
            Self::Lever | Self::LeverEu | Self::External => token.to_owned(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "greenhouse" => Self::Greenhouse,
            "lever" => Self::Lever,
            "lever_eu" => Self::LeverEu,
            "ashby" => Self::Ashby,
            "external" => Self::External,
            _ => return None,
        })
    }
}

impl std::fmt::Display for Ats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Primary key of a posting (§4.2). For `External` postings `board_token`
/// is the source that reported it (a feed id or `manual`) and `job_id` is
/// that source's item id or a hash of the URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct PostingKey {
    pub ats: Ats,
    pub board_token: String,
    pub job_id: String,
}
