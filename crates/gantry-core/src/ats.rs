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

    /// ATSes whose public board API Gantry polls.
    pub const BOARD_APIS: [Self; 4] = [Self::Greenhouse, Self::Lever, Self::LeverEu, Self::Ashby];
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
