use crate::{
    protocol::{ScrollMessage, ScrollMessagePayload, ScrollWireEvent, ScrollWireProtocolState},
    ScrollWireConfig,
};
use alloy_primitives::Signature;
use alloy_rlp::BytesMut;
use futures::{Stream, StreamExt};
use reth_eth_wire::multiplex::ProtocolConnection;
use reth_network::Direction;
use reth_network_api::PeerId;
use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::trace;

mod handler;
pub(crate) use handler::ScrollConnectionHandler;

/// Connection between two peers using the scroll wire protocol.
#[derive(Debug)]
pub struct ScrollWireConnection {
    /// The inbound connection from the peer.
    pub conn: ProtocolConnection,
    /// The direction of the connection.
    pub direction: Direction,
    /// A stream of messages to be announced to the peer.
    pub outbound: UnboundedReceiverStream<ScrollMessage>,
    /// A channel to emit events.
    pub events: UnboundedSender<ScrollWireEvent>,
    /// The peer id of the connection.
    pub peer_id: PeerId,
}

impl ScrollWireConnection {
    /// Creates a new [`ScrollWireConnection`] with the provided [`ProtocolConnection`] and
    /// [`ScrollWireProtocolState`].
    pub fn new(
        peer_id: PeerId,
        conn: ProtocolConnection,
        direction: Direction,
        outbound: UnboundedReceiver<ScrollMessage>,
        state: ScrollWireProtocolState,
    ) -> Self {
        Self {
            conn,
            direction,
            outbound: outbound.into(),
            events: state.event_sender().clone(),
            peer_id,
        }
    }
}

impl Stream for ScrollWireConnection {
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            // We send all the messages in the outbound channel to the peer.
            if let Poll::Ready(Some(msg)) = this.outbound.poll_next_unpin(cx) {
                trace!(target: "scroll::wire::connection", peer_id = %this.peer_id, ?msg, "broadcasting message");
                return Poll::Ready(Some(msg.encoded()));
            }

            // We receive a message from the peer, if the message is `None` then we shutdown the
            // connection.
            let Some(msg) = ready!(this.conn.poll_next_unpin(cx)) else {
                return Poll::Ready(None);
            };

            // We decode the message, if the message can not be decoded then we disconnect.
            let Some(msg) = ScrollMessage::decode(&mut &msg[..]) else {
                return Poll::Ready(None);
            };

            // We handle the message.
            match msg.payload {
                ScrollMessagePayload::NewBlock(new_block) => {
                    // If the signature can be decoded then we send a new block event.
                    trace!(target: "scroll::wire::connection", peer_id = %this.peer_id, block = ?new_block.block.hash_slow(), "Received new block from peer");
                    if let Ok(signature) = Signature::from_raw(&new_block.signature[..]) {
                        this.events
                            .send(ScrollWireEvent::NewBlock {
                                block: new_block.block,
                                signature,
                                peer_id: this.peer_id,
                            })
                            .unwrap();
                    } else {
                        // If the signature can not be decoded then we disconnect.
                        trace!(target: "scroll::wire::connection", peer_id = %this.peer_id, "Received new block with invalid signature from peer - dropping the scroll-wire connection." );
                        return Poll::Ready(None);
                    }
                }
            }
        }
    }
}
