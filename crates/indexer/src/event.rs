use alloy_primitives::B256;
use rollup_node_primitives::{BatchInfo, BlockInfo, L2BlockInfoWithL1Messages};

/// An event emitted by the indexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexerEvent {
    /// A `BatchCommit` event has been indexed returning the batch info and the L2 block info to
    /// revert to due to a batch revert.
    BatchCommitIndexed {
        /// The batch info.
        batch_info: BatchInfo,
        /// The L1 block number in which the batch was committed.
        l1_block_number: u64,
        /// The safe L2 block info.
        safe_head: Option<BlockInfo>,
    },
    /// A `BatchFinalization` event has been indexed returning the batch hash and new finalized L2
    /// block.
    BatchFinalizationIndexed(B256, Option<BlockInfo>),
    /// A `Finalized` event has been indexed returning the block number and new finalized L2
    /// block.
    FinalizedIndexed(u64, Option<BlockInfo>),
    /// A `L1Message` event has been indexed returning the message queue index.
    L1MessageIndexed(u64),
    /// A `Unwind` event has been indexed returning the reorg block number.
    UnwindIndexed {
        /// The L1 block number of the new L1 head.
        l1_block_number: u64,
        /// The L1 message queue index of the new L1 head.
        queue_index: Option<u64>,
        /// The L2 head block info.
        l2_head_block_info: Option<BlockInfo>,
        /// The L2 safe block info.
        l2_safe_block_info: Option<BlockInfo>,
    },
    /// A block has been indexed returning batch and block info.
    BlockIndexed(L2BlockInfoWithL1Messages, Option<BatchInfo>),
}
