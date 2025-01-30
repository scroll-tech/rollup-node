use reth_network::{
    import::{BlockImport as RethBlockImport, BlockValidation},
    NetworkPrimitives,
};
use reth_network_peers::PeerId;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_scroll_primitives::ScrollBlock;
use scroll_wire::Event;
use secp256k1::ecdsa::Signature;
use std::{
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{trace, warn};

const ECDSA_SIGNATURE_LEN: usize = 64;

/// A block import implementation for the eth-wire protocol that sends block to the scroll-wire
/// protocol.
///
/// The block import implementation delegates the block import to the inner block import and then
/// sends the block to the scroll-wire protocol if the block is valid.
#[derive(Debug)]
pub struct BridgeBlockImport {
    /// A sender for sending events to the scroll-wire protocol.
    events: UnboundedSender<Event>,
    /// The inner block import.
    inner: Box<dyn RethBlockImport<reth_scroll_primitives::ScrollBlock>>,
}

impl BridgeBlockImport {
    /// Creates a new [`BridgeBlockImport`] instance with the provided events sender and inner block
    /// import.
    pub fn new(
        events: UnboundedSender<Event>,
        inner_block_import: Box<dyn RethBlockImport<reth_scroll_primitives::ScrollBlock>>,
    ) -> Self {
        Self { events, inner: inner_block_import }
    }

    /// Bridges a new block from the eth-wire protocol to the scroll-wire protocol.
    fn bridge_new_block_to_scroll_wire(
        &self,
        peer_id: PeerId,
        block: Arc<reth_eth_wire_types::NewBlock<ScrollBlock>>,
    ) {
        // We create a reference to the extra data of the incoming block.
        let extra_data = &block.block.extra_data;

        // If we can extract a signature from the extra data we send the block to the scroll-wire
        // protocol. The signature is extracted from the last `ECDSA_SIGNATURE_LEN` bytes of the
        // extra data field.
        if let Some(signature) = extra_data
            .len()
            .checked_sub(ECDSA_SIGNATURE_LEN)
            .and_then(|i| Signature::from_compact(&extra_data[i..]).ok())
        {
            let block = block.block.clone();
            trace!(target: "bridge::import", peer_id = %peer_id, block = ?block, "Received new block from eth-wire protocol");

            // We trigger a new block event to be sent to the network manager. If this results in an
            // error it means the network manager has been dropped.
            let _ = self.events.send(Event::NewBlock { peer_id, block, signature });
        } else {
            warn!(target: "bridge::import", peer_id = %peer_id, "Failed to extract signature from block extra data");
        }
    }
}

impl RethBlockImport<reth_scroll_primitives::ScrollBlock> for BridgeBlockImport {
    /// This function is called when a new block is received from the network, it delegates the
    /// block import to the inner block import.
    fn on_new_block(
        &mut self,
        peer_id: PeerId,
        incoming_block: reth_network::message::NewBlockMessage<
            <ScrollNetworkPrimitives as NetworkPrimitives>::Block,
        >,
    ) {
        // We then delegate the block import to the inner block import.
        self.inner.on_new_block(peer_id, incoming_block);
    }

    /// This function is called when the block import is polled.
    ///
    /// If the block import is ready we check if the block is valid and if it is we send the block
    /// to the scroll-wire protocol and then return the outcome.
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        reth_network::import::BlockImportOutcome<
            <ScrollNetworkPrimitives as NetworkPrimitives>::Block,
        >,
    > {
        if let Poll::Ready(outcome) = self.inner.poll(cx) {
            match outcome.result {
                Ok(BlockValidation::ValidBlock { ref block }) |
                Ok(BlockValidation::ValidHeader { ref block }) => {
                    self.bridge_new_block_to_scroll_wire(outcome.peer, block.block.clone());
                    return Poll::Ready(outcome)
                }
                Err(_) => Poll::Ready(outcome),
            }
        } else {
            return Poll::Pending;
        }
    }
}
