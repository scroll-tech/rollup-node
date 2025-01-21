use crate::{
    protocol::{Event, Message, MessagePayload, ProtocolState},
    ScrollWireConfig,
};
use alloy_rlp::BytesMut;
use futures::{Stream, StreamExt};
use reth_eth_wire::multiplex::ProtocolConnection;
use reth_network::Direction;
use reth_network_api::PeerId;
use secp256k1::ecdsa::Signature;
use std::{
    pin::Pin,
    task::Poll,
    task::{ready, Context},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;

mod handler;
pub use handler::ConnectionHandler;

/// Connection between two peers using the ScrollWire protocol.
pub struct Connection {
    /// The inbound connection from the peer.
    pub conn: ProtocolConnection,
    /// The direction of the connection.
    pub direction: Direction,
    /// A stream of messages to be announced to the peer.
    pub outbound: UnboundedReceiverStream<Message>,
    /// A channel to emit events.
    pub events: UnboundedSender<Event>,
    /// The peer id of the connection.
    pub peer_id: PeerId,
}

impl Connection {
    /// Creates a new [`ScrollWireProtocolConnection`] with the provided connection and state.
    pub fn new(
        peer_id: PeerId,
        conn: ProtocolConnection,
        direction: Direction,
        outbound: UnboundedReceiver<Message>,
        state: ProtocolState,
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

impl Stream for Connection {
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

            // We decode the message, if the message can not be decoded then we disconnect.
            let Some(msg) = Message::decode(&mut &msg[..]) else {
                return Poll::Ready(None);
            };

            // We handle the message.
            match msg.payload {
                MessagePayload::NewBlock(new_block) => {
                    // If the signature is can be decoded then we send a new block event.
                    if let Some(signature) = Signature::from_compact(&new_block.signature[..]).ok()
                    {
                        this.events
                            .send(Event::NewBlock {
                                block: new_block.block,
                                signature,
                                peer_id: this.peer_id,
                            })
                            .unwrap();
                    } else {
                        // If the signature can not be decoded then we disconnect.
                        return Poll::Ready(None);
                    }
                }
            }
        }
    }
}
