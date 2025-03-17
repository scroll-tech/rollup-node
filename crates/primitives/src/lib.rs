//! Primitive types for the Rollup Node.

#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(not(feature = "std"))]
extern crate alloc as std;

pub use block::BlockInfo;
mod block;

pub use batch::{BatchInput, BatchInputBuilder, BatchInputV1, BatchInputV2};
mod batch;

pub use transaction::L1MessageWithBlockNumber;
mod transaction;
