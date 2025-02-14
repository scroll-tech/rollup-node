//! Engine Driver for the Scroll Rollup Node. The [`EngineDriver`] exposes the main interface for
//! the Rollup Node to the Engine API.

mod block_info;
pub use block_info::BlockInfo;

mod engine;
pub use engine::EngineDriver;

mod error;
pub use error::EngineDriverError;

mod fcs;
pub use fcs::ForkchoiceState;

mod payload;
pub use payload::ExecutionPayloadProvider;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
