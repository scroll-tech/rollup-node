use crate::protocol::{
    ScrollWireEvent, ScrollWireMessage, ScrollWireMessageKind, ScrollWireProtocolState,
};
use alloy_primitives::PrimitiveSignature;
use alloy_rlp::BytesMut;
use futures::{Stream, StreamExt};
use reth_eth_wire::multiplex::ProtocolConnection;
use reth_network::Direction;
use reth_network_api::PeerId;
use std::{
    pin::Pin,
    task::Poll,
    task::{ready, Context},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;

mod handler;
pub use handler::ScrollWireConnectionHandler;

/// Connection between two peers using the ScrollWire protocol.
pub struct ScrollWireProtocolConnection {
    /// The inbound connection from the peer.
    pub conn: ProtocolConnection,
    /// The direction of the connection.
    pub direction: Direction,
    /// A stream of messages to be announced to the peer.
    pub outbound: UnboundedReceiverStream<ScrollWireMessage>,
    /// A channel to emit events.
    pub events: UnboundedSender<ScrollWireEvent>,
    /// The peer id of the connection.
    pub peer_id: PeerId,
}

impl ScrollWireProtocolConnection {
    /// Creates a new [`ScrollWireProtocolConnection`] with the provided connection and state.
    pub fn new(
        peer_id: PeerId,
        conn: ProtocolConnection,
        direction: Direction,
        outbound: UnboundedReceiver<ScrollWireMessage>,
        state: ScrollWireProtocolState,
    ) -> Self {
        Self {
            conn,
            direction,
            outbound: outbound.into(),
            events: state.events().clone(),
            peer_id,
        }
    }
}

impl Stream for ScrollWireProtocolConnection {
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            // We announce the message to the peer.
            if let Poll::Ready(Some(msg)) = this.outbound.poll_next_unpin(cx) {
                println!("broadcasting message: {:?}", msg);
                return Poll::Ready(Some(msg.encoded()));
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
                    // If the signature is valid then we announce the block to the network.
                    if let Some(signature) =
                        TryInto::<PrimitiveSignature>::try_into(&new_block.signature[..]).ok()
                    {
                        this.events
                            .send(ScrollWireEvent::NewBlock {
                                block: new_block.block,
                                signature,
                                peer_id: this.peer_id,
                            })
                            .unwrap();
                    } else {
                        // If the signature is invalid then we disconnect.
                        return Poll::Ready(None);
                    }
                }
            }
        }
    }
}
