use reth_network::{import::BlockImport as RethBlockImport, NetworkPrimitives};
use reth_network_peers::PeerId;
use scroll_wire::{Event, NewBlock};
use secp256k1::ecdsa::Signature;
use std::collections::VecDeque;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{trace, warn};

const ECDSA_SIGNATURE_LEN: usize = 64;

/// A block import implementation for the eth-wire protocol that sends block to the scroll-wire
/// protocol and then delegates the block import to the inner block import.
#[derive(Debug)]
pub struct BridgeBlockImport {
    /// A sender for sending events to the scroll-wire protocol.
    events: UnboundedSender<Event>,
    /// The inner block import.
    inner: Box<dyn RethBlockImport>,
}

impl BridgeBlockImport {
    /// Creates a new [`BridgeBlockImport`] instance with the provided events sender and inner block
    /// import.
    pub fn new(
        events: UnboundedSender<Event>,
        inner_block_import: Box<dyn RethBlockImport>,
    ) -> Self {
        Self { events, inner: inner_block_import }
    }
}

impl RethBlockImport for BridgeBlockImport {
    /// This function is called when a new block is received from the network.
    ///
    /// It extracts the signature of the block from the extra data field and sends the block to the
    /// scroll-wire protocol. It then delegates the block import to the inner block import.
    fn on_new_block(
        &mut self,
        peer_id: PeerId,
        incoming_block: reth_network::message::NewBlockMessage<
            <reth_network::EthNetworkPrimitives as NetworkPrimitives>::Block,
        >,
    ) {
        // We create a reference to the extra data of the incoming block.
        let extra_data = &incoming_block.block.block.extra_data;

        // If we can extract a signature from the extra data we send the block to the scroll-wire
        // protocol. The signature is extracted from the last `ECDSA_SIGNATURE_LEN` bytes of the
        // extra data field.
        if let Some(signature) = extra_data
            .len()
            .checked_sub(ECDSA_SIGNATURE_LEN)
            .and_then(|i| Signature::from_compact(&extra_data[i..]).ok())
        {
            let block = incoming_block.block.block.clone();
            trace!(target: "bridge::import", peer_id = %peer_id, block = ?block, "Received new block from eth-wire protocol");

            // We trigger a new block event to be sent to the network manager. If this results in an
            // error it means the network manager has been dropped.
            let _ = self.events.send(Event::NewBlock { peer_id, block, signature });
        } else {
            warn!(target: "bridge::import", peer_id = %peer_id, "Failed to extract signature from block extra data");
        }

        // We then delegate the block import to the inner block import.
        self.inner.on_new_block(peer_id, incoming_block);
    }

    /// This function is called when the block import is polled.
    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<
        reth_network::import::BlockImportOutcome<
            <reth_network::EthNetworkPrimitives as NetworkPrimitives>::Block,
        >,
    > {
        // We delegate the polling to the inner block import.
        self.inner.poll(cx)
    }
}

/// A block import type that always returns a valid block.
#[derive(Debug, Default)]
pub struct ValidBlockImport {
    /// A buffer for storing the blocks that are received.
    blocks: VecDeque<(PeerId, NewBlock)>,
    waker: Option<std::task::Waker>,
}

impl scroll_network::BlockImport for ValidBlockImport {
    fn on_new_block(
        &mut self,
        peer_id: PeerId,
        block: reth_primitives::Block,
        signature: Signature,
    ) {
        trace!(target: "network::import::ValidBlockImport", peer_id = %peer_id, block = ?block, "Received new block");
        self.blocks.push_back((peer_id, NewBlock::new(signature, block)));

        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<scroll_network::BlockImportOutcome> {
        // If there are blocks in the buffer we return the first block.
        if let Some((peer, new_block)) = self.blocks.pop_front() {
            std::task::Poll::Ready(scroll_network::BlockImportOutcome {
                peer,
                result: Ok(scroll_network::BlockValidation::ValidBlock { new_block }),
            })
        } else {
            self.waker = Some(cx.waker().clone());
            std::task::Poll::Pending
        }
    }
}
