use scroll_network::NewBlockWithPeer;

/// An event that can be emitted by the rollup node manager.
#[derive(Debug, Clone)]
pub enum RollupEvent {
    /// A new block has been received from the network.
    NewBlockReceived(NewBlockWithPeer),
}
