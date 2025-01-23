use crate::protocol::Message;
use reth_network::Direction;
use reth_network_api::PeerId;
use secp256k1::ecdsa::Signature;
use tokio::sync::mpsc::UnboundedSender;

/// The events that can be emitted by the ScrollWire protocol.
#[derive(Debug)]
pub enum Event {
    /// A new connection has been established.
    ConnectionEstablished {
        /// The direction of the connection.
        direction: Direction,
        /// The peer id of the connection.
        peer_id: PeerId,
        /// A sender for sending messages to the connection.
        to_connection: UnboundedSender<Message>,
    },
    /// A new block received from the network
    NewBlock {
        /// The peer id the block was received from.
        peer_id: PeerId,
        /// The block that was received.
        block: reth_primitives::Block,
        /// The signature of the block.
        signature: Signature,
    },
}

impl Event {
    /// Creates a new [`Event::ConnectionEstablished`] event.
    pub fn connection_established(
        direction: Direction,
        peer_id: PeerId,
        to_connection: UnboundedSender<Message>,
    ) -> Self {
        Self::ConnectionEstablished {
            direction,
            peer_id,
            to_connection,
        }
    }

    /// Creates a new [`Event::NewBlock`] event.
    pub fn new_block(peer_id: PeerId, block: reth_primitives::Block, signature: Signature) -> Self {
        Self::NewBlock {
            peer_id,
            block,
            signature,
        }
    }
}
