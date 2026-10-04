//! Discovery (§4.1–4.2): Tier 1 sources of §4.1.1, the manual URL
//! resolver, offline geocoding, and the run that ties them to the store and
//! the deterministic filters.
//!
//! Every network request goes through [`http::Fetcher`], which enforces
//! the politeness rules. Unit and fixture tests never touch the network.

pub mod bundled;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod geocode;
pub mod html;
pub mod http;
pub mod pipeline;
pub mod probe;
pub mod resolver;
pub mod robots;
pub mod sources;

pub use pipeline::{DiscoverInputs, DiscoverReport, discover};
