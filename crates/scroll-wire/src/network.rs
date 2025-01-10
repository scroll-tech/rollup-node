use crate::protocol::{NewBlockMessage, ScrollWireEvent};
use futures::StreamExt;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::protocol::{ProtoEvents, ToPeers};

/// A manager for the ScrollWire protocol.
pub struct ScrollNetwork {
    events: UnboundedReceiverStream<ScrollWireEvent>,
    new_block_announcements: ToPeers,
    from_handle_rx: UnboundedReceiverStream<ScrollNetworkHandleMessage>,
    handle: ScrollNetworkHandle,
}

impl ScrollNetwork {
    /// Creates a new `ScrollNetwork` instance.
    pub fn new(events: ProtoEvents, new_block_announcements: ToPeers) -> Self {
        println!("Creating new ScrollNetwork instance");
        let (to_network_tx, from_handle_rx) = mpsc::unbounded_channel();
        let handle = ScrollNetworkHandle::new(to_network_tx);
        Self {
            events: events.into(),
            new_block_announcements,
            from_handle_rx: from_handle_rx.into(),
            handle,
        }
    }

    /// Returns the handle to the network.
    pub fn handle(&self) -> &ScrollNetworkHandle {
        &self.handle
    }

    /// Announces a new block to the network.
    fn announce_block(&self, new_block_message: NewBlockMessage) {
        if self.new_block_announcements.receiver_count() == 0 {
            println!("No subscribers to new block announcements, skipping announcement");
            return;
        }

        self.new_block_announcements
            .send(new_block_message)
            .unwrap();
    }
}

impl Future for ScrollNetwork {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Process messages from the network handle.
        while let Poll::Ready(new_block_message) = this.from_handle_rx.poll_next_unpin(cx) {
            match new_block_message {
                Some(ScrollNetworkHandleMessage::AnnounceBlock(new_block_message)) => {
                    this.announce_block(new_block_message);
                }
                None => break,
            }
        }

        // Process events from the network.
        while let Poll::Ready(new_block) = this.events.poll_next_unpin(cx) {
            match new_block {
                Some(ScrollWireEvent::NewBlock {
                    block,
                    signature: _,
                }) => {
                    // We announce the block to the network.
                    println!("Received new block from the network: {:?}", block);
                }
                None => break,
            }
        }

        Poll::Pending
    }
}

/// A handle to the ScrollWire network.
#[derive(Debug, Clone)]
pub struct ScrollNetworkHandle {
    to_network_tx: UnboundedSender<ScrollNetworkHandleMessage>,
}

impl ScrollNetworkHandle {
    /// Creates a new `ScrollNetworkHandle` instance.
    pub fn new(to_network_tx: UnboundedSender<ScrollNetworkHandleMessage>) -> Self {
        Self { to_network_tx }
    }

    /// Announces a new block to the network.
    pub fn announce_block(&self, new_block_message: NewBlockMessage) {
        self.to_network_tx
            .send(ScrollNetworkHandleMessage::AnnounceBlock(new_block_message))
            .unwrap();
    }
}

/// Messages that can be sent to the NetworkManager from the NetworkHandle.
pub enum ScrollNetworkHandleMessage {
    AnnounceBlock(NewBlockMessage),
}
