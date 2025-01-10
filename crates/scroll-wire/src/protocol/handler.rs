use super::proto::NewBlockMessage;
use super::ScrollWireEvent;
use crate::connection::ScrollWireConnectionHandler;
use reth_network::protocol::ProtocolHandler;
use reth_network_api::PeerId;
use tokio::sync::{broadcast, mpsc};

pub type ProtoEvents = mpsc::UnboundedReceiver<ScrollWireEvent>;
pub type ToPeers = tokio::sync::broadcast::Sender<NewBlockMessage>;

const BROADCAST_CHANNEL_SIZE: usize = 200;

#[derive(Debug, Clone)]
pub struct ScrollWireProtocolState {
    pub events: mpsc::UnboundedSender<ScrollWireEvent>,
    pub to_peers: broadcast::Sender<NewBlockMessage>,
}

#[derive(Debug)]
pub struct ScrollWireProtocolHandler {
    pub state: ScrollWireProtocolState,
}

impl ScrollWireProtocolHandler {
    pub fn new() -> (Self, ProtoEvents, ToPeers) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (to_peers_tx, _) = broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let state = ScrollWireProtocolState {
            events: events_tx,
            to_peers: to_peers_tx.clone(),
        };
        (Self { state }, events_rx, to_peers_tx)
    }
}

impl ProtocolHandler for ScrollWireProtocolHandler {
    type ConnectionHandler = ScrollWireConnectionHandler;

    fn on_incoming(&self, _socket_addr: std::net::SocketAddr) -> Option<Self::ConnectionHandler> {
        Some(ScrollWireConnectionHandler {
            state: self.state.clone(),
        })
    }

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
