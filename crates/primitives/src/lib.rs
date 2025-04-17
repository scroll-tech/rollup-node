//! Primitive types for the Rollup Node.

#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(not(feature = "std"))]
extern crate alloc as std;

pub use attributes::ScrollPayloadAttributesWithBatchInfo;
mod attributes;

pub use block::BlockInfo;
mod block;

pub use batch::{BatchCommitData, BatchInfo};
mod batch;

pub use bounded_vec::BoundedVec;
mod bounded_vec;

pub use transaction::L1MessageWithBlockNumber;
mod transaction;
