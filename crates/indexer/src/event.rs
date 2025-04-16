use alloy_primitives::B256;
use rollup_node_primitives::{BatchInfo, BlockInfo};

/// An event emitted by the indexer.
#[derive(Debug, Clone, Copy)]
pub enum IndexerEvent {
    /// A `BatchCommit` event has been indexed returning the batch info.
    BatchCommitIndexed(BatchInfo),
    /// A `BatchFinalization` event has been indexed returning the batch hash and new finalized L2
    /// block.
    BatchFinalizationIndexed(B256, Option<BlockInfo>),
    /// A `Finalized` event has been indexed returning the block number and new finalized L2
    /// block.
    FinalizedIndexed(u64, Option<BlockInfo>),
    /// A `L1Message` event has been indexed returning the message queue index.
    L1MessageIndexed(u64),
    /// A `Reorg` event has been indexed returning the reorg block number.
    ReorgIndexed(u64),
    /// A `BatchToBlock` event has been indexed returning batch and block info.
    BatchToBlockIndexed(BatchInfo, BlockInfo),
}
