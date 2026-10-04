use gantry_discovery::resolver::{Resolved, resolve};
use schemars::JsonSchema;
use serde::Serialize;

use crate::{CmdError, Context};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UrlAdded {
    pub url: String,
    pub resolved: Resolved,
    /// `false` if this URL was added before.
    pub added: bool,
    /// What the next run will do with it.
    pub next: String,
}

/// `gantry url add`: resolves a posting or board URL and records it; the
/// next run polls the board it belongs to. Makes no request.
pub fn add_url(ctx: &Context, url: &str, now: i64) -> Result<UrlAdded, CmdError> {
    let resolved = resolve(url).map_err(|e| CmdError::InvalidInput(e.to_string()))?;
    let next = match &resolved {
        Resolved::Posting {
            ats, board_token, ..
        }
        | Resolved::Board { ats, board_token } => {
            format!("the next run polls the {ats} board \"{board_token}\"")
        }
        Resolved::GreenhouseEmbed { host, .. } => format!(
            "Greenhouse board embedded on {host}; the next run looks for its board token by name"
        ),
        Resolved::External { .. } => "not a supported board API; it is kept for manual paste \
             mode (M6), and nothing is fetched from it"
            .to_owned(),
    };
    let store = ctx.open_store()?;
    let added = store.add_manual_url(
        url.trim(),
        &serde_json::to_string(&resolved).map_err(gantry_store::StoreError::from)?,
        now,
    )?;
    Ok(UrlAdded {
        url: url.trim().to_owned(),
        resolved,
        added,
        next,
    })
}
