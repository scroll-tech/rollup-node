use alloy_primitives::B256;
use rollup_node_primitives::{BatchInfo, BlockInfo, L2BlockInfoWithL1Messages};

/// An event emitted by the indexer.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    ReorgIndexed {
        /// The L1 block number of the new L1 head.
        l1_block_number: u64,
        /// The L1 message queue index of the new L1 head.
        queue_index: Option<u64>,
        /// The L2 block info of the new L2 head.
        l2_block_info: Option<BlockInfo>,
    },
    /// A block has been indexed returning batch and block info.
    BlockIndexed(L2BlockInfoWithL1Messages, Option<BatchInfo>),
}
