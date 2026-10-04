use gantry_discovery::http::{Politeness, ReqwestTransport, Transport};
use gantry_discovery::{DiscoverInputs, DiscoverReport, bundled, discover};

use crate::{CmdError, Context};

/// `gantry run --discover-only`: one discovery run against the network.
pub async fn run_discover(ctx: &Context, now: i64) -> Result<DiscoverReport, CmdError> {
    let transport = ReqwestTransport::new().map_err(CmdError::Network)?;
    run_discover_with(ctx, &transport, Politeness::default(), now).await
}

/// A discovery run over any transport; tests pass saved fixtures.
pub async fn run_discover_with(
    ctx: &Context,
    transport: &dyn Transport,
    politeness: Politeness,
    now: i64,
) -> Result<DiscoverReport, CmdError> {
    let snapshot = ctx.snapshot()?;
    let store = ctx.open_store()?;
    let seed = bundled::seed_companies(&snapshot.search.occupations)?;
    let feeds = bundled::feeds()?;
    let inputs = DiscoverInputs {
        snapshot: &snapshot,
        seed: &seed,
        feeds: &feeds,
        now,
    };
    Ok(discover(&store, transport, politeness, inputs).await?)
}
