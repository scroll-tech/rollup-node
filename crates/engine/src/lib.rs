//! Engine Driver for the Scroll Rollup Node. The [`EngineDriver`] exposes the main interface for
//! the Rollup Node to the Engine API.

pub(crate) mod api;

pub use driver::EngineDriver;
mod driver;

pub use error::EngineDriverError;
mod error;

pub use event::EngineDriverEvent;
mod event;

pub use fcs::{genesis_hash_from_chain_spec, ForkchoiceState};
mod fcs;

mod future;
pub use future::ConsolidationOutcome;

pub use metrics::EngineDriverMetrics;
mod metrics;

mod payload;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
