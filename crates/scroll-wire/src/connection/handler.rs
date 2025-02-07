use super::{ScrollWireConfig, ScrollWireConnection};
use crate::{
    protocol::{ProtocolState, ScrollMessage},
    ScrollWireEvent,
};
use reth_network::protocol::{ConnectionHandler as ConnectionHandlerTrait, OnNotSupported};
use tracing::trace;

/// The connection handler for the `ScrollWire` protocol.
#[derive(Debug)]
pub struct ScrollConnectionHandler {
    state: ProtocolState,
    config: ScrollWireConfig,
}

impl ScrollConnectionHandler {
    /// Creates a new [`ScrollConnectionHandler`] with the provided state and config.
    pub const fn from_parts(state: ProtocolState, config: ScrollWireConfig) -> Self {
        Self { state, config }
    }
}

impl ConnectionHandlerTrait for ScrollConnectionHandler {
    type Connection = ScrollWireConnection;

    /// The protocol that this connection handler is for.
    fn protocol(&self) -> reth_eth_wire::protocol::Protocol {
        ScrollMessage::protocol()
    }

    /// Called when a incoming connection is invoked by a peer.
    fn on_unsupported_by_peer(
        self,
        _supported: &reth_eth_wire::capability::SharedCapabilities,
        _direction: reth_network::Direction,
        _peer_id: reth_network_api::PeerId,
    ) -> OnNotSupported {
        if self.config.connect_unsupported_peer() {
            trace!(target: "scroll_wire::connection::handler", "Peer does not support the ScrollWire protocol, keeping connection alive.");
            OnNotSupported::KeepAlive
        } else {
            trace!(target: "scroll_wire::connection::handler", "Peer does not support the ScrollWire protocol, disconnecting.");
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
        trace!(target: "scroll_wire::connection::handler", peer_id = %peer_id, direction = ?direction, "Connection established with peer");

        // Create a new channel for sending messages to the connection.
        let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel();

        // Emit a ConnectionEstablished containing the sender to send messages to the connection.
        self.state
            .event_sender()
            .send(ScrollWireEvent::ConnectionEstablished {
                direction,
                peer_id,
                to_connection: msg_tx,
            })
            .expect("Failed to send ConnectionEstablished event - receiver dropped");

        ScrollWireConnection::new(peer_id, conn, direction, msg_rx, self.state)
    }
}
