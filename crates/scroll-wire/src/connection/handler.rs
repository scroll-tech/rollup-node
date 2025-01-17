use super::ScrollWireProtocolConnection;
use crate::{
    protocol::{ScrollWireMessage, ScrollWireProtocolState},
    ScrollWireEvent,
};
use reth_network::protocol::{ConnectionHandler, OnNotSupported};

/// The connection handler for the ScrollWire protocol.
pub struct ScrollWireConnectionHandler {
    pub state: ScrollWireProtocolState,
}

impl ConnectionHandler for ScrollWireConnectionHandler {
    type Connection = ScrollWireProtocolConnection;

    /// The protocol that this connection handler is for.
    fn protocol(&self) -> reth_eth_wire::protocol::Protocol {
        ScrollWireMessage::protocol()
    }

    /// Called when a incoming connection is invoked by a peer.
    fn on_unsupported_by_peer(
        self,
        _supported: &reth_eth_wire::capability::SharedCapabilities,
        _direction: reth_network::Direction,
        _peer_id: reth_network_api::PeerId,
    ) -> OnNotSupported {
        // If the peer does not support the scroll-wire protocol then we disconnect.
        OnNotSupported::Disconnect
    }

    /// Called when a connection is established with a peer.
    fn into_connection(
        self,
        direction: reth_network::Direction,
        peer_id: reth_network_api::PeerId,
        conn: reth_eth_wire::multiplex::ProtocolConnection,
    ) -> Self::Connection {
        println!("Connection established with peer for the scroll-wire protocol");
        let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel();
        self.state
            .events()
            .send(ScrollWireEvent::ConnectionEstablished {
                direction,
                peer_id,
                to_connection: msg_tx,
            })
            .expect("Failed to send ConnectionEstablished event");
        ScrollWireProtocolConnection::new(peer_id, conn, direction, msg_rx, self.state)
    }
}
