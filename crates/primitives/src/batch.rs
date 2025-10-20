use super::L2BlockInfoWithL1Messages;

use alloy_primitives::{Bytes, B256};
use std::{sync::Arc, vec::Vec};

/// The batch information.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct BatchInfo {
    /// The index of the batch.
    pub index: u64,
    /// The hash of the batch.
    pub hash: B256,
}

impl BatchInfo {
    /// Returns a new instance of [`BatchInfo`].
    pub const fn new(index: u64, hash: B256) -> Self {
        Self { index, hash }
    }
}

impl std::fmt::Display for BatchInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BatchInfo {{ index: {}, hash: 0x{} }}", self.index, self.hash)
    }
}

/// The input data for a batch.
///
/// This is used as input for the derivation pipeline. All data remains in its raw serialized form.
/// The data is then deserialized, enriched and processed in the derivation pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchCommitData {
    /// The hash of the committed batch.
    pub hash: B256,
    /// The index of the batch.
    pub index: u64,
    /// The block number in which the batch was committed.
    pub block_number: u64,
    /// The block timestamp in which the batch was committed.
    pub block_timestamp: u64,
    /// The commit transaction calldata.
    pub calldata: Arc<Bytes>,
    /// The optional blob hash for the commit.
    pub blob_versioned_hash: Option<B256>,
    /// The block number at which the batch finalized event was emitted.
    pub finalized_block_number: Option<u64>,
}

impl From<BatchCommitData> for BatchInfo {
    fn from(value: BatchCommitData) -> Self {
        Self { index: value.index, hash: value.hash }
    }
}

/// The outcome of consolidating a batch with the L2 chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchConsolidationOutcome {
    /// The batch info for the consolidated batch.
    pub batch_info: BatchInfo,
    /// The consolidation outcomes for each block in the batch.
    pub blocks: Vec<L2BlockInfoWithL1Messages>,
}

impl BatchConsolidationOutcome {
    /// Creates a new empty batch consolidation outcome for the given batch info.
    pub const fn new(batch_info: BatchInfo) -> Self {
        Self { batch_info, blocks: Vec::new() }
    }

    /// Pushes a block consolidation outcome to the batch.
    pub fn push_block(&mut self, block: L2BlockInfoWithL1Messages) {
        self.blocks.push(block);
    }
}

/// The outcome of consolidating a block with the L2 chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockConsolidationOutcome {
    /// The derived block was already part of the chain, update the fork choice state.
    UpdateFcs(L2BlockInfoWithL1Messages),
    /// The fork choice state was already ahead of the derived block.
    Skipped(L2BlockInfoWithL1Messages),
    /// The derived block resulted in a reorg of the L2 chain.
    Reorged(L2BlockInfoWithL1Messages),
}

impl BlockConsolidationOutcome {
    /// Returns the block info with l2 messages for the consolidated block.
    pub const fn block_info(&self) -> &L2BlockInfoWithL1Messages {
        match self {
            Self::UpdateFcs(info) | Self::Skipped(info) | Self::Reorged(info) => info,
        }
    }

    /// Consumes the outcome and returns the block info with l2 messages for the consolidated block.
    pub fn into_inner(self) -> L2BlockInfoWithL1Messages {
        match self {
            Self::UpdateFcs(info) | Self::Skipped(info) | Self::Reorged(info) => info,
        }
    }
}

impl std::fmt::Display for BlockConsolidationOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UpdateFcs(info) => {
                write!(f, "Update Fcs to block {}", info.block_info.number)
            }
            Self::Skipped(info) => write!(f, "Skipped block {}", info.block_info.number),
            Self::Reorged(attrs) => {
                write!(f, "Reorged to block {}", attrs.block_info)
            }
        }
    }
}

#[cfg(feature = "arbitrary")]
mod arbitrary_impl {
    use super::*;

    impl arbitrary::Arbitrary<'_> for BatchCommitData {
        fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
            let batch_index = u.arbitrary::<u32>()? as u64;
            let batch_hash = u.arbitrary::<B256>()?;
            let block_number = u.arbitrary::<u32>()? as u64;
            let block_timestamp = u.arbitrary::<u32>()? as u64;
            let bytes = u.arbitrary::<Bytes>()?;
            let blob_hash = u.arbitrary::<bool>()?.then_some(u.arbitrary::<B256>()?);

            Ok(Self {
                hash: batch_hash,
                index: batch_index,
                block_number,
                block_timestamp,
                calldata: Arc::new(bytes),
                blob_versioned_hash: blob_hash,
                finalized_block_number: None,
            })
        }
    }
}
