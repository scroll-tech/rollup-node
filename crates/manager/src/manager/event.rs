use alloy_primitives::B256;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_indexer::ChainOrchestratorEvent;
use rollup_node_primitives::{BatchInfo, ChainImport, L2BlockInfoWithL1Messages};
use rollup_node_signer::SignerEvent;
use scroll_db::L1MessageStart;
use scroll_engine::ConsolidationOutcome;
use scroll_network::NewBlockWithPeer;

/// An event that can be emitted by the rollup node manager.
#[derive(Debug, Clone)]
pub enum RollupManagerEvent {
    /// A new block has been received from the network.
    NewBlockReceived(NewBlockWithPeer),
    /// New block sequenced.
    BlockSequenced(ScrollBlock),
    /// New block imported.
    BlockImported(ScrollBlock),
    /// Consolidated block derived from L1.
    L1DerivedBlockConsolidated(ConsolidationOutcome),
    /// An L1 message with the given index has been indexed.
    L1MessageIndexed(u64),
    /// A new event from the signer.
    SignerEvent(SignerEvent),
    /// An event from the chain orchestrator.
    ChainOrchestratorEvent(ChainOrchestratorEvent),
    /// An error occurred consolidating the L1 messages.
    L1MessageConsolidationError {
        /// The expected L1 messages hash.
        expected: B256,
        /// The actual L1 messages hash.
        actual: B256,
    },
    /// A block has been received containing an L1 message that is not in the database.
    L1MessageMissingInDatabase {
        /// The L1 message start index or hash.
        start: L1MessageStart,
    },
    /// An optimistic sync has been triggered by the chain orchestrator.
    OptimisticSyncTriggered(ScrollBlock),
    /// A chain extension has been triggered by the chain orchestrator.
    ChainExtensionTriggered(ChainImport),
    /// An L2 chain has been committed.
    L2ChainCommitted(L2BlockInfoWithL1Messages, Option<BatchInfo>, bool),
    /// The L1 watcher has synced to the tip of the L1 chain.
    L1Synced,
}
