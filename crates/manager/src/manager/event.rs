use reth_scroll_primitives::ScrollBlock;
use rollup_node_signer::SignerEvent;
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
}
