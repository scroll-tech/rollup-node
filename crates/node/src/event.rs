use rollup_node_primitives::BlockInfo;
use scroll_network::NewBlockWithPeer;

/// An event that can be emitted by the rollup node manager.
#[derive(Debug, Clone)]
pub enum RollupEvent {
    /// A new block has been received from the network.
    NewBlockReceived(NewBlockWithPeer),
    /// New Block Sequenced.
    NewBlockSequenced(NewBlockWithPeer),
    /// Consolidated block derived from L1.
    L1DerivedBlockConsolidated(BlockInfo),
}
