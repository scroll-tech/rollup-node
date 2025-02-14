use reth_network_api::PeerId;
use reth_scroll_primitives::ScrollBlock;
use secp256k1::ecdsa::Signature;

/// A new block with the peer id that it was received from.
#[derive(Debug, Clone)]
pub struct NewBlockWithPeer {
    pub peer_id: PeerId,
    pub block: ScrollBlock,
    pub signature: Signature,
}

/// An event that is emitted by the network manager to its subscribers.
#[derive(Debug)]
pub enum NetworkManagerEvent {
    NewBlock(NewBlockWithPeer),
}
