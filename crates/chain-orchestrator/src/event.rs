use alloy_consensus::Header;
use alloy_primitives::{Signature, B256};
use reth_network_peers::PeerId;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::{
    BatchConsolidationOutcome, BatchInfo, BlockConsolidationOutcome, BlockInfo, ChainImport,
    L2BlockInfoWithL1Messages,
};
use scroll_db::L1MessageKey;
use scroll_network::NewBlockWithPeer;

/// An event emitted by the `ChainOrchestrator`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainOrchestratorEvent {
    /// A received block failed the consensus checks.
    BlockFailedConsensusChecks(B256, PeerId),
    /// A new block has been received from the network but we have insufficient data to process it
    /// due to being in optimistic mode.
    InsufficientDataForReceivedBlock(B256),
    /// The block that we have received is already known.
    BlockAlreadyKnown(B256, PeerId),
    /// A fork of the chain that is older than the current chain has been received.
    OldForkReceived {
        /// The headers of the old fork.
        headers: Vec<Header>,
        /// The peer that provided the old fork.
        peer_id: PeerId,
        /// The signature of the old fork.
        signature: Signature,
    },
    /// The chain should be optimistically synced to the provided block.
    OptimisticSync(BlockInfo),
    /// The chain has been extended, returning the new blocks.
    ChainExtended(ChainImport),
    /// The chain has reorged, returning the new chain and the peer that provided them.
    ChainReorged(ChainImport),
    /// A `BatchCommit` event has been indexed returning the batch info and L1 block number at
    /// which the event was emitted. If this event is associated with a batch revert then the
    /// `safe_head` will also be populated with the `BlockInfo` that represents the new L2 head.
    BatchCommitIndexed {
        /// The batch info.
        batch_info: BatchInfo,
        /// The L1 block number in which the batch was committed.
        l1_block_number: u64,
        /// The safe L2 block info.
        safe_head: Option<BlockInfo>,
    },
    /// A batch has been finalized returning a list of finalized batches.
    BatchFinalized(u64, Vec<BatchInfo>),
    /// A new L1 block has been received returning the L1 block number.
    NewL1Block(u64),
    /// An L1 block has been finalized returning the L1 block number and the list of finalized
    /// batches.
    L1BlockFinalized(u64, Vec<BatchInfo>),
    /// A `L1Message` event has been committed returning the message queue index.
    L1MessageCommitted(u64),
    /// A reorg has occurred on L1, returning the L1 block number of the new L1 head,
    /// the L1 message queue index of the new L1 head, and optionally the L2 head and safe block
    /// info if the reorg resulted in a new L2 head or safe block.
    L1Reorg {
        /// The L1 block number of the new L1 head.
        l1_block_number: u64,
        /// The L1 message queue index of the new L1 head.
        queue_index: Option<u64>,
        /// The L2 head block info.
        l2_head_block_info: Option<BlockInfo>,
        /// The L2 safe block info.
        l2_safe_block_info: Option<BlockInfo>,
    },
    /// The chain orchestrator has synced to the L1 head.
    L1Synced,
    /// An L2 block has been committed returning the [`L2BlockInfoWithL1Messages`] and an
    /// optional [`BatchInfo`] if the block is associated with a committed batch.
    L2ChainCommitted(L2BlockInfoWithL1Messages, Option<BatchInfo>, bool),
    /// An L2 consolidated block has been committed returning the [`L2BlockInfoWithL1Messages`].
    L2ConsolidatedBlockCommitted(L2BlockInfoWithL1Messages),
    /// A new block has been sequenced by the sequencer.
    BlockSequenced(ScrollBlock),
    /// A new block has been signed by the signer.
    SignedBlock {
        /// The signed block.
        block: ScrollBlock,
        /// The signature of the signed block.
        signature: Signature,
    },
    /// An L1 message was not found in the database.
    L1MessageNotFoundInDatabase(L1MessageKey),
    /// A new block has been received from the network.
    NewBlockReceived(NewBlockWithPeer),
    /// A batch consolidation has occurred, returning the outcome of the consolidation.
    BatchConsolidated(BatchConsolidationOutcome),
    /// An L1 derived block has been consolidated, returning the outcome of the consolidation.
    BlockConsolidated(BlockConsolidationOutcome),
}
