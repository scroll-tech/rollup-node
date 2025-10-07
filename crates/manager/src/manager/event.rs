use alloy_primitives::B256;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use rollup_node_signer::SignerEvent;
use rollup_node_watcher::L1Notification;
use scroll_db::L1MessageKey;
use scroll_engine::ConsolidationOutcome;
use scroll_network::NewBlockWithPeer;

/// An event that can be emitted by the rollup node manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollupManagerEvent {
    /// A new block has been received from the network.
    NewBlockReceived(NewBlockWithPeer),
    /// New block sequenced.
    BlockSequenced(ScrollBlock),
    /// New block imported.
    BlockImported(ScrollBlock),
    /// Consolidated block derived from L1.
    L1DerivedBlockConsolidated(ConsolidationOutcome),
    /// A new event from the signer.
    SignerEvent(SignerEvent),
    /// A reorg event.
    Reorg(u64),
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
        /// The L1 message key.
        key: L1MessageKey,
    },
    /// An event was received from the L1 watcher.
    L1NotificationEvent(L1Notification),
}
