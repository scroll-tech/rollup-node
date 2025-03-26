//! Engine Driver for the Scroll Rollup Node. The [`EngineDriver`] exposes the main interface for
//! the Rollup Node to the Engine API.

pub use engine::EngineDriver;
mod engine;

pub use error::EngineDriverError;
mod error;

pub use fcs::ForkchoiceState;
mod fcs;

mod payload;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
