use crate::protocol::{Event, Message, NewBlock};
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

/// A manager for the ScrollWire protocol.
#[derive(Debug)]
pub struct ScrollWireManager {
    /// A stream of [`Event`]s produced by the scroll wire protocol.
    events: UnboundedReceiverStream<Event>,
    /// A map of connections to peers.
    connections: HashMap<PeerId, UnboundedSender<Message>>,
    /// A map of the state of the scroll wire protocol. Currently the state for each peer
    /// is just a cache of the last 100 blocks seen by each peer.
    state: HashMap<PeerId, LruCache<B256>>,
}

impl ScrollWireManager {
    /// Creates a new [`ScrollWireManager`] instance.
    pub fn new(events: UnboundedReceiver<Event>) -> Self {
        trace!(target: "scroll_wire::manager", "Creating new ScrollWireManager instance");
        Self { events: events.into(), connections: HashMap::new(), state: HashMap::new() }
    }

    /// Announces a new block to the specified peer.
    pub fn announce_block(&mut self, peer_id: PeerId, block: &NewBlock, hash: B256) {
        if let Entry::Occupied(to_connection) = self.connections.entry(peer_id) {
            // We send the block to the peer. If we receive an error we remove the peer from the
            // connections map and delete its state as the connection is no longer valid.
            if let Err(_) = to_connection.get().send(Message::new_block(block.clone())) {
                trace!(target: "scroll_wire::manager", peer_id = %peer_id, "Failed to send block to peer - dropping peer.");
                self.state.remove(&peer_id);
                to_connection.remove();
            } else {
                // Upon successful sending of the block we update the state of the peer.
                trace!(target: "scroll_wire::manager", peer_id = %peer_id, "Announced block to peer");
                self.state
                    .entry(peer_id)
                    .or_insert_with(|| LruCache::new(LRU_CACHE_SIZE))
                    .insert(hash);
            }
        }
    }

    /// Returns the state of the ScrollWire protocol.
    pub fn state(&self) -> &HashMap<PeerId, LruCache<B256>> {
        &self.state
    }

    /// Returns a mutable reference to the state of the ScrollWire protocol.
    pub fn state_mut(&mut self) -> &mut HashMap<PeerId, LruCache<B256>> {
        &mut self.state
    }
}

impl Future for ScrollWireManager {
    type Output = Event;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Process events from the network.
        while let Poll::Ready(new_block) = this.events.poll_next_unpin(cx) {
            match new_block {
                Some(Event::NewBlock { peer_id, block, signature }) => {
                    // We announce the block to the network.
                    println!("Received new block with signature [{signature:?}] from the network: {block:?} ");
                    return Poll::Ready(Event::NewBlock { peer_id, block, signature });
                }
                Some(Event::ConnectionEstablished { direction, peer_id, to_connection }) => {
                    println!(
                        "Established connection with peer: {:?} for direction: {:?}",
                        peer_id, direction
                    );
                    this.connections.insert(peer_id, to_connection);
                    this.state.insert(peer_id, LruCache::new(100));
                }
                None => break,
            }
        }

        Poll::Pending
    }
}
