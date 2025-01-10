use crate::protocol::{
    NewBlockMessage, ScrollWireEvent, ScrollWireMessage, ScrollWireMessageKind,
    ScrollWireProtocolState,
};
use alloy_rlp::BytesMut;
use futures::{Stream, StreamExt};
use reth_eth_wire::multiplex::ProtocolConnection;
use std::{
    pin::Pin,
    task::Poll,
    task::{ready, Context},
};
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::wrappers::BroadcastStream;

/// Connection between two peers using the ScrollWire protocol.
pub struct ScrollWireProtocolConnection {
    /// The inbound connection from the peer.
    pub conn: ProtocolConnection,
    /// A stream of blocks to be announced to the peer.
    pub block_announcements: BroadcastStream<NewBlockMessage>,
    /// A channel to emit events.
    pub events: UnboundedSender<ScrollWireEvent>,
}

impl ScrollWireProtocolConnection {
    /// Creates a new `ScrollWireProtocolConnection` with the provided connection and state.
    pub fn new(conn: ProtocolConnection, state: ScrollWireProtocolState) -> Self {
        let block_announcements = state.to_peers.subscribe().into();
        let events = state.events.clone();
        Self {
            conn,
            block_announcements,
            events,
        }
    }
}

impl Stream for ScrollWireProtocolConnection {
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            // We announce the block to the peer.
            if let Poll::Ready(Some(Ok(new_block))) = this.block_announcements.poll_next_unpin(cx) {
                println!("broadcasting new block: {:?}", new_block);
                return Poll::Ready(Some(ScrollWireMessage::new_block(new_block).encoded()));
            }

            // We receive a message from the peer.
            let Some(msg) = ready!(this.conn.poll_next_unpin(cx)) else {
                return Poll::Ready(None);
            };

            // We decode the message.
            let Some(msg) = ScrollWireMessage::decode(&mut &msg[..]) else {
                return Poll::Ready(None);
            };

            // We handle the message.
            match msg.message {
                ScrollWireMessageKind::NewBlock(new_block) => {
                    // We emit a new block event.
                    this.events
                        .send(ScrollWireEvent::NewBlock {
                            block: new_block.block,
                            signature: (&new_block.signature[..])
                                .try_into()
                                .expect("msg signature"),
                        })
                        .unwrap();
                }
            }
        }
    }
}
