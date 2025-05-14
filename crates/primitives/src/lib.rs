//! Primitive types for the Rollup Node.

#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(not(feature = "std"))]
extern crate alloc as std;

mod attributes;
pub use attributes::ScrollPayloadAttributesWithBatchInfo;

mod block;
pub use block::{BlockInfo, L2BlockInfoWithL1Messages};

mod batch;
pub use batch::{BatchCommitData, BatchInfo};

mod bounded_vec;
pub use bounded_vec::BoundedVec;

mod node;
pub use node::{
    config::{NodeConfig, MAINNET_SYSTEM_CONTRAT_ADDRESS, SEPOLIA_SYSTEM_CONTRAT_ADDRESS},
    consensus::ConsensusUpdate,
};

mod signature;
pub use signature::sig_encode_hash;

mod transaction;

pub use transaction::L1MessageEnvelope;
