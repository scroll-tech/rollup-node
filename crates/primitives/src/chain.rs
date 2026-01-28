use alloy_primitives::Signature;
use alloy_rpc_types_engine::ForkchoiceUpdated;
use reth_network_peers::PeerId;
use reth_scroll_primitives::ScrollBlock;
use std::vec::Vec;

/// A structure representing a chain import, which includes a vector of blocks,
/// the peer ID from which the blocks were received, and a signature for the import of the chain
/// tip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainImport {
    /// The blocks that are part of the chain import.
    pub chain: Vec<ScrollBlock>,
    /// The peer ID from which the blocks were received.
    pub peer_id: PeerId,
    /// The signature for the import of the chain tip.
    pub signature: Signature,
    /// The result of the chain import operation.
    pub result: ForkchoiceUpdated,
}

impl ChainImport {
    /// Creates a new `ChainImport` instance with the provided blocks, peer ID, and signature.
    pub const fn new(
        blocks: Vec<ScrollBlock>,
        peer_id: PeerId,
        signature: Signature,
        result: ForkchoiceUpdated,
    ) -> Self {
        Self { chain: blocks, peer_id, signature, result }
    }
}
