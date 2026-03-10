use crate::{
    error::AnnounceBlockError,
    protocol::{NewBlock, ScrollMessage, ScrollWireEvent},
};
use futures::StreamExt;
use reth_network_api::PeerId;
use std::{
    collections::{hash_map::Entry, HashMap},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::trace;

/// The size of the LRU cache used to track blocks that have been seen by peers.
pub const LRU_CACHE_SIZE: u32 = 100;

/// A manager for the `ScrollWire` protocol.
#[derive(Debug)]
pub struct ScrollWireManager {
    /// A stream of [`ScrollWireEvent`]s produced by the scroll wire protocol.
    events: UnboundedReceiverStream<ScrollWireEvent>,
    /// A map of connections to peers.
    connections: HashMap<PeerId, UnboundedSender<ScrollMessage>>,
}

impl ScrollWireManager {
    /// Creates a new [`ScrollWireManager`] instance.
    pub fn new(events: UnboundedReceiver<ScrollWireEvent>) -> Self {
        trace!(target: "scroll::wire::manager", "Creating new ScrollWireManager instance");
        Self { events: events.into(), connections: HashMap::new() }
    }

    /// Announces a new block to the specified peer.
    pub fn announce_block(
        &mut self,
        peer_id: PeerId,
        block: &NewBlock,
    ) -> Result<(), AnnounceBlockError> {
        let Entry::Occupied(to_connection) = self.connections.entry(peer_id) else {
            return Err(AnnounceBlockError::PeerNotConnected(peer_id));
        };

        if to_connection.get().send(ScrollMessage::new_block(block.clone())).is_err() {
            trace!(target: "scroll::wire::manager", peer_id = %peer_id, "Failed to send block to peer - dropping peer.");
            to_connection.remove();
            return Err(AnnounceBlockError::SendFailed(peer_id));
        }

        trace!(target: "scroll::wire::manager", peer_id = %peer_id, "Announced block to peer");
        Ok(())
    }

    /// Returns an iterator over the connected peer IDs.
    pub fn connected_peers(&self) -> impl Iterator<Item = &PeerId> {
        self.connections.keys()
    }

    /// Checks if a peer is connected to the manager via scroll-wire.
    pub fn is_connected(&self, peer_id: PeerId) -> bool {
        self.connections.contains_key(&peer_id)
    }
}

impl Future for ScrollWireManager {
    type Output = ScrollWireEvent;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Process events from the network.
        while let Poll::Ready(new_block) = this.events.poll_next_unpin(cx) {
            match new_block {
                Some(ScrollWireEvent::NewBlock { peer_id, block, signature }) => {
                    // We announce the block to the network.
                    trace!(target: "scroll::wire::manager", "Received new block with signature [{signature:?}] from the network: {:?} ", block.hash_slow());
                    return Poll::Ready(ScrollWireEvent::NewBlock { peer_id, block, signature });
                }
                Some(ScrollWireEvent::ConnectionEstablished {
                    direction,
                    peer_id,
                    to_connection,
                }) => {
                    trace!(
                        target: "scroll::wire::manager",
                        peer_id = %peer_id,
                        direction = ?direction,
                        "Established connection with peer: {:?} for direction: {:?}",
                        peer_id,
                        direction
                    );
                    this.connections.insert(peer_id, to_connection);
                }
                None => break,
            }
        }

        Poll::Pending
    }
}
