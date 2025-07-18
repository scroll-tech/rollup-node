//! Primitive types for the Rollup Node.

#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(not(feature = "std"))]
extern crate alloc as std;

mod attributes;
pub use attributes::ScrollPayloadAttributesWithBatchInfo;

mod block;
pub use block::{BlockInfo, L2BlockInfoWithL1Messages, DEFAULT_BLOCK_DIFFICULTY};

mod batch;
pub use batch::{BatchCommitData, BatchInfo};

mod bounded_vec;
pub use bounded_vec::BoundedVec;

mod chain;
pub use chain::ChainImport;

mod metadata;
pub use metadata::Metadata;

#[cfg(feature = "std")]
mod metrics;
#[cfg(feature = "std")]
pub use metrics::MeteredFuture;

mod node;
pub use node::{
    config::{
        NodeConfig, DEVNET_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS,
        DEVNET_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS, DEVNET_ROLLUP_CONTRACT_ADDRESS,
        DEV_L1_START_BLOCK_NUMBER, DEV_SYSTEM_CONTRAT_ADDRESS,
        MAINNET_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS, MAINNET_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS,
        MAINNET_L1_START_BLOCK_NUMBER, MAINNET_ROLLUP_CONTRACT_ADDRESS,
        MAINNET_SYSTEM_CONTRAT_ADDRESS, SEPOLIA_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS,
        SEPOLIA_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS, SEPOLIA_L1_START_BLOCK_NUMBER,
        SEPOLIA_ROLLUP_CONTRACT_ADDRESS, SEPOLIA_SYSTEM_CONTRAT_ADDRESS,
    },
    consensus::ConsensusUpdate,
};

mod signature;
pub use signature::sig_encode_hash;

mod transaction;
pub use transaction::L1MessageEnvelope;
