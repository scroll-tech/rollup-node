use crate::protocol::Message;
use reth_network::Direction;
use reth_network_api::PeerId;
use secp256k1::ecdsa::Signature;
use tokio::sync::mpsc::UnboundedSender;

/// The events that can be emitted by the ScrollWire protocol.
#[derive(Debug)]
pub enum Event {
    ConnectionEstablished {
        direction: Direction,
        peer_id: PeerId,
        to_connection: UnboundedSender<Message>,
    },
    NewBlock {
        peer_id: PeerId,
        block: reth_primitives::Block,
        signature: Signature,
    },
}
