use crate::protocol::{NewBlock, ScrollMessage, ScrollWireEvent};
use alloy_primitives::B256;
use futures::StreamExt;
use reth_network::cache::LruCache;
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
    /// Tracks block hashes received from each peer for duplicate detection.
    peer_state: HashMap<PeerId, LruCache<B256>>,
}

impl ScrollWireManager {
    /// Creates a new [`ScrollWireManager`] instance.
    pub fn new(events: UnboundedReceiver<ScrollWireEvent>) -> Self {
        trace!(target: "scroll::wire::manager", "Creating new ScrollWireManager instance");
        Self { events: events.into(), connections: HashMap::new(), peer_state: HashMap::new() }
    }

    /// Announces a new block to the specified peer.
    pub fn announce_block(&mut self, peer_id: PeerId, block: &NewBlock) {
        if let Entry::Occupied(to_connection) = self.connections.entry(peer_id) {
            // We send the block to the peer. If we receive an error we remove the peer from the
            // connections map and peer_block_state as the connection is no longer valid.
            if to_connection.get().send(ScrollMessage::new_block(block.clone())).is_err() {
                trace!(target: "scroll::wire::manager", peer_id = %peer_id, "Failed to send block to peer - dropping peer.");
                self.peer_state.remove(&peer_id);
                to_connection.remove();
            } else {
                trace!(target: "scroll::wire::manager", peer_id = %peer_id, "Announced block to peer");
            }
        }
    }

    /// Returns an iterator over the connected peer IDs.
    pub fn connected_peers(&self) -> impl Iterator<Item = &PeerId> {
        self.connections.keys()
    }

    /// Returns a reference to the peer state map.
    pub const fn peer_state(&self) -> &HashMap<PeerId, LruCache<B256>> {
        &self.peer_state
    }

    /// Returns a mutable reference to the peer state map.
    pub const fn peer_state_mut(&mut self) -> &mut HashMap<PeerId, LruCache<B256>> {
        &mut self.peer_state
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
