use crate::protocol::{NewBlock, ScrollWireEvent, ScrollWireMessage};
use alloy_primitives::B256;
use futures::StreamExt;
use reth_network::cache::LruCache;
use reth_network_api::PeerId;
use std::collections::{hash_map::Entry, HashMap};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;

/// A manager for the ScrollWire protocol.
pub struct ScrollWireManager {
    /// A stream of [`ScrollWireEvent`]s produced by the scroll wire protocol.
    events: UnboundedReceiverStream<ScrollWireEvent>,
    /// A map of connections to peers.
    connections: HashMap<PeerId, UnboundedSender<ScrollWireMessage>>,
    /// A map of the state of the scroll wire protocol. Currently the state for each peer
    /// is just a cache of the last 100 blocks seen by each peer.
    state: HashMap<PeerId, LruCache<B256>>,
}

impl ScrollWireManager {
    /// Creates a new [`ScrollWireManager`] instance.
    pub fn new(events: UnboundedReceiver<ScrollWireEvent>) -> Self {
        println!("Creating new ScrollNetwork instance");
        Self {
            events: events.into(),
            connections: HashMap::new(),
            state: HashMap::new(),
        }
    }

    /// Announces a new block to the specified peer.
    pub fn announce_block(&mut self, peer_id: PeerId, block: NewBlock, hash: B256) {
        if let Entry::Occupied(to_connection) = self.connections.entry(peer_id) {
            // if the channel to the peer is closed then we can remove the connection and state from the manager.
            let connection = to_connection.get();
            if connection.is_closed() {
                self.state.remove(&peer_id);
                to_connection.remove();
                return;
            }

            // We send the block to the peer.
            to_connection
                .get()
                .send(ScrollWireMessage::new_block(block))
                .unwrap();

            // We update the state of the peer.
            let state = self
                .state
                .entry(peer_id)
                .or_insert_with(|| LruCache::new(100));
            state.insert(hash);
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
    type Output = ScrollWireEvent;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Process events from the network.
        while let Poll::Ready(new_block) = this.events.poll_next_unpin(cx) {
            match new_block {
                Some(ScrollWireEvent::NewBlock {
                    peer_id,
                    block,
                    signature,
                }) => {
                    // We announce the block to the network.
                    println!("Received new block with signature [{signature:?}] from the network: {block:?} ");
                    return Poll::Ready(ScrollWireEvent::NewBlock {
                        peer_id,
                        block,
                        signature,
                    });
                }
                Some(ScrollWireEvent::ConnectionEstablished {
                    direction,
                    peer_id,
                    to_connection,
                }) => {
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
