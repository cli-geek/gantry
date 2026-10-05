use std::fs::TryLockError;

use gantry_discovery::http::{Politeness, ReqwestTransport, Transport};
use gantry_discovery::{DiscoverInputs, DiscoverReport, bundled, discover};
use gantry_platform::PlatformError;

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
    // Two overlapping runs would double the request rate and count one
    // omission as two misses. The OS releases the lock if the process dies.
    let lock_path = ctx.paths.data_dir().join("run.lock");
    let io_error = |source| PlatformError::Io {
        path: lock_path.clone(),
        source,
    };
    // Not truncated: on Windows, truncating a file another process has
    // locked fails, which would read as an error instead of "busy".
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(io_error)?;
    gantry_platform::restrict_file(&lock_path)?;
    lock.try_lock().map_err(|e| match e {
        TryLockError::WouldBlock => CmdError::Busy,
        TryLockError::Error(source) => io_error(source).into(),
    })?;
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
