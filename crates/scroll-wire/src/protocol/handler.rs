use super::ScrollWireEvent;
use crate::connection::ScrollWireConnectionHandler;
use reth_network::protocol::ProtocolHandler;
use reth_network_api::PeerId;
use tokio::sync::mpsc;

/// A Receiver for ScrollWireEvents.
pub type ScrollWireEventReceiver = mpsc::UnboundedReceiver<ScrollWireEvent>;

/// A Sender for ScrollWireEvents.
pub type ScrollWireEventSender = mpsc::UnboundedSender<ScrollWireEvent>;

/// The state of the ScrollWire protocol.
///
/// This contains a sender for emitting [`ScrollWireEvent`]s.
#[derive(Debug, Clone)]
pub struct ScrollWireProtocolState {
    events: ScrollWireEventSender,
}

impl ScrollWireProtocolState {
    /// Returns a clone of the sender for emitting [`ScrollWireEvent`]s.
    pub fn events(&self) -> &ScrollWireEventSender {
        &self.events
    }
}

/// A handler for the ScrollWire protocol.
///
/// This handler contains the state of the protocol ([`ScrollWireProtocolState`]).
/// This type is responsible for handling incoming and outgoing connections. It would typically be
/// used for protocol negotiation, but currently we do not have any.
#[derive(Debug)]
pub struct ScrollWireProtocolHandler {
    pub state: ScrollWireProtocolState,
}

impl ScrollWireProtocolHandler {
    /// Creates a tuple of ([`ScrollWireProtocolHandler`], [`ScrollWireEventReceiver`]).
    pub fn new() -> (Self, ScrollWireEventReceiver) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let state = ScrollWireProtocolState { events: events_tx };
        (Self { state }, events_rx)
    }
}

impl ProtocolHandler for ScrollWireProtocolHandler {
    type ConnectionHandler = ScrollWireConnectionHandler;

    /// Called when a incoming connection is invoked by a peer.
    fn on_incoming(&self, _socket_addr: std::net::SocketAddr) -> Option<Self::ConnectionHandler> {
        Some(ScrollWireConnectionHandler {
            state: self.state.clone(),
        })
    }

    /// Called when a connection is established with a peer.
    fn on_outgoing(
        &self,
        _socket_addr: std::net::SocketAddr,
        _peer_id: PeerId,
    ) -> Option<Self::ConnectionHandler> {
        Some(ScrollWireConnectionHandler {
            state: self.state.clone(),
        })
    }
}
