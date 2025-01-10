use super::ScrollWireProtocolConnection;
use crate::protocol::{ScrollWireMessage, ScrollWireProtocolState};
use reth_network::protocol::{ConnectionHandler, OnNotSupported};

pub struct ScrollWireConnectionHandler {
    pub state: ScrollWireProtocolState,
}

impl ConnectionHandler for ScrollWireConnectionHandler {
    type Connection = ScrollWireProtocolConnection;

    fn protocol(&self) -> reth_eth_wire::protocol::Protocol {
        ScrollWireMessage::protocol()
    }

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
        _direction: reth_network::Direction,
        _peer_id: reth_network_api::PeerId,
        conn: reth_eth_wire::multiplex::ProtocolConnection,
    ) -> Self::Connection {
        println!("Connection established with peer for the scroll-wire protocol");
        ScrollWireProtocolConnection::new(conn, self.state)
    }
}
