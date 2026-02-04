use reth_network_api::PeerId;

/// Errors that can occur when announcing a block.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AnnounceBlockError {
    /// The peer is not connected.
    #[error("Peer {0} is not connected")]
    PeerNotConnected(PeerId),
    /// Failed to send the block to the peer.
    #[error("Failed to send block to peer {0}")]
    SendFailed(PeerId),
}
