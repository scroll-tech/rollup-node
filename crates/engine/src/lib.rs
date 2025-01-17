//! Engine Driver for the Scroll Rollup Node. The [`EngineDriver`] exposes the main interface for
//! the Rollup Node to the Engine API.

mod block_info;
pub use block_info::BlockInfo;

mod engine;
pub use engine::EngineDriver;

mod payload;
pub use payload::{ExecutionPayloadProvider, ScrollPayloadAttributes};

#[cfg(any(test, feature = "test_utils"))]
mod test_utils;
