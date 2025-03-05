//! Primitive types for the Rollup Node.

pub use block::BlockInfo;
mod block;

pub use batch::{BatchInput, BatchInputBuilder, BatchInputV1, BatchInputV2};
mod batch;

pub use transaction::L1MessageWithBlockNumber;
mod transaction;
