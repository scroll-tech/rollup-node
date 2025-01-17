use crate::protocol::ScrollWireMessage;
use alloy_primitives::PrimitiveSignature;
use reth_network::Direction;
use reth_network_api::PeerId;
use tokio::sync::mpsc::UnboundedSender;

/// The events that can be emitted by the ScrollWire protocol.
#[derive(Debug)]
pub enum ScrollWireEvent {
    ConnectionEstablished {
        direction: Direction,
        peer_id: PeerId,
        to_connection: UnboundedSender<ScrollWireMessage>,
    },
    NewBlock {
        peer_id: PeerId,
        block: reth_primitives::Block,
        signature: PrimitiveSignature,
    },
}
