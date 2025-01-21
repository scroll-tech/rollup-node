use super::{Connection, ScrollWireConfig};
use crate::{
    protocol::{Message, ProtocolState},
    Event,
};
use reth_network::protocol::{ConnectionHandler as ConnectionHandlerTrait, OnNotSupported};

/// The connection handler for the ScrollWire protocol.
pub struct ConnectionHandler {
    pub state: ProtocolState,
    config: ScrollWireConfig,
}

impl ConnectionHandler {
    /// Creates a new [`ScrollWireConnectionHandler`] with the provided state and config.
    pub fn from_parts(state: ProtocolState, config: ScrollWireConfig) -> Self {
        Self { state, config }
    }
}

impl ConnectionHandlerTrait for ConnectionHandler {
    type Connection = Connection;

    /// The protocol that this connection handler is for.
    fn protocol(&self) -> reth_eth_wire::protocol::Protocol {
        Message::protocol()
    }

    /// Called when a incoming connection is invoked by a peer.
    fn on_unsupported_by_peer(
        self,
        _supported: &reth_eth_wire::capability::SharedCapabilities,
        _direction: reth_network::Direction,
        _peer_id: reth_network_api::PeerId,
    ) -> OnNotSupported {
        if self.config.connect_unsupported_peer() {
            OnNotSupported::KeepAlive
        } else {
            OnNotSupported::Disconnect
        }
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
            .send(Event::ConnectionEstablished {
                direction,
                peer_id,
                to_connection: msg_tx,
            })
            .expect("Failed to send ConnectionEstablished event");
        Connection::new(peer_id, conn, direction, msg_rx, self.state)
    }
}
