//! A stateless derivation pipeline for Scroll.
//!
//! This crate provides a simple implementation of a derivation pipeline that transforms commit
//! payload into payload attributes for block building.

pub use error::DerivationPipelineError;
mod error;

pub use hash::compute_data_hash;
mod hash;

use alloy_primitives::B256;
use rollup_node_primitives::{BatchInput, L1MessageWithBlockNumber};
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_codec::{decoding::batch_header::BatchHeader, Codec, CommitDataSource};
use scroll_l1::abi::calls::CommitBatchCall;

pub trait L1MessageProvider {
    fn next_message(&self) -> L1MessageWithBlockNumber;
}

/// Returns an iterator over a tuple of batch hash [`B256`] and [`ScrollPayloadAttributes`] from the
/// [`CommitDataSource`] and a [`L1MessageProvider`].
pub fn derive<P: L1MessageProvider>(
    batch: BatchInput,
    l1_message_provider: &P,
) -> Result<impl Iterator<Item = (B256, ScrollPayloadAttributes)>, DerivationPipelineError> {
    Ok(std::iter::empty())
}
