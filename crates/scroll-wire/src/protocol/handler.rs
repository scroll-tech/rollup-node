use super::Event;
use crate::{connection::ConnectionHandler, ScrollWireConfig};
use reth_network::protocol::ProtocolHandler as ProtocolHandlerTrait;
use reth_network_api::PeerId;
use tokio::sync::mpsc;

/// A Receiver for ScrollWireEvents.
pub type ScrollWireEventReceiver = mpsc::UnboundedReceiver<Event>;

/// A Sender for ScrollWireEvents.
pub type ScrollWireEventSender = mpsc::UnboundedSender<Event>;

/// The state of the protocol.
///
/// This contains a sender for emitting [`Event`]s.
#[derive(Debug, Clone)]
pub struct ProtocolState {
    /// A sender for emitting [`Event`]s.
    event_sender: ScrollWireEventSender,
}

impl ProtocolState {
    /// Returns a reference to the sender for emitting [`ScrollWireEvent`]s.
    pub fn event_sender(&self) -> &ScrollWireEventSender {
        &self.event_sender
    }
}

/// A handler for the ScrollWire protocol.
///
/// This handler contains the state of the protocol ([`ProtocolState`]) and protocol configuration.
/// This type is responsible for handling incoming and outgoing connections. It would typically be
/// used for protocol negotiation, but currently we do not have any.
#[derive(Debug)]
pub struct ProtocolHandler {
    state: ProtocolState,
    config: ScrollWireConfig,
}

impl ProtocolHandler {
    /// Creates a tuple of ([`ProtocolHandler`], [`ScrollWireEventReceiver`]) from the provided configuration.
    pub fn new(config: ScrollWireConfig) -> (Self, ScrollWireEventReceiver) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let state = ProtocolState {
            event_sender: events_tx,
        };
        (Self { state, config }, events_rx)
    }

    /// Creates a new [`ProtocolHandler`] with the provided state and config.
    pub fn from_parts(state: ProtocolState, config: ScrollWireConfig) -> Self {
        Self { state, config }
    }
}

impl ProtocolHandlerTrait for ProtocolHandler {
    type ConnectionHandler = ConnectionHandler;

    /// Called when a incoming connection is invoked by a peer.
    fn on_incoming(&self, _socket_addr: std::net::SocketAddr) -> Option<Self::ConnectionHandler> {
        Some(ConnectionHandler::from_parts(
            self.state.clone(),
            self.config.clone(),
        ))
    }

    /// Called when a connection is established with a peer.
    fn on_outgoing(
        &self,
        _socket_addr: std::net::SocketAddr,
        _peer_id: PeerId,
    ) -> Option<Self::ConnectionHandler> {
        Some(ConnectionHandler::from_parts(
            self.state.clone(),
            self.config.clone(),
        ))
    }
}
