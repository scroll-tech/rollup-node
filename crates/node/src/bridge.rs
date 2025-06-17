// TODO: remove this when re-integrating the bridge
#![allow(dead_code)]

use alloy_primitives::Signature;
use reth_network::import::{BlockImport as RethBlockImport, NewBlockEvent};
use reth_network_peers::PeerId;
use reth_scroll_primitives::ScrollBlock;
use scroll_network::NewBlockWithPeer;
use std::{
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{trace, warn};

const ECDSA_SIGNATURE_LEN: usize = 65;

/// A block import implementation for the eth-wire protocol that sends block to the scroll-wire
/// protocol.
#[derive(Debug)]
struct BridgeBlockImport {
    /// A sender for sending events to the scroll-wire protocol.
    new_block_tx: UnboundedSender<NewBlockWithPeer>,
}

impl BridgeBlockImport {
    /// Creates a new [`BridgeBlockImport`] instance with the provided events sender and inner block
    /// import.
    const fn new(new_block_tx: UnboundedSender<NewBlockWithPeer>) -> Self {
        Self { new_block_tx }
    }

    /// Bridges a new block from the eth-wire protocol to the scroll-wire protocol.
    fn bridge_new_block_to_scroll_wire(&self, peer_id: PeerId, block: Arc<ScrollBlock>) {
        // We create a reference to the extra data of the incoming block.
        let extra_data = &block.extra_data;

        // If we can extract a signature from the extra data we send the block to the scroll-wire
        // protocol. The signature is extracted from the last `ECDSA_SIGNATURE_LEN` bytes of the
        // extra data field.
        if let Some(signature) = extra_data
            .len()
            .checked_sub(ECDSA_SIGNATURE_LEN)
            .and_then(|i| Signature::from_raw(&extra_data[i..]).ok())
        {
            let block = &*block;
            trace!(target: "scroll::bridge::import", peer_id = %peer_id, block = ?block.hash_slow(), "Received new block from eth-wire protocol");

            // We trigger a new block event to be sent to the rollup node's network manager. If this
            // results in an error it means the network manager has been dropped.
            let _ = self.new_block_tx.send(NewBlockWithPeer {
                peer_id,
                block: block.clone(),
                signature,
            });
        } else {
            warn!(target: "scroll::bridge::import", peer_id = %peer_id, "Failed to extract signature from block extra data");
        }
    }
}

impl RethBlockImport<ScrollBlock> for BridgeBlockImport {
    /// This function is called when a new block is received from the network, it delegates the
    /// block import to the inner block import.
    fn on_new_block(&mut self, peer_id: PeerId, incoming_block: NewBlockEvent<ScrollBlock>) {
        // We then delegate the block import to the inner block import.
        match incoming_block {
            NewBlockEvent::Block(block) => {
                self.bridge_new_block_to_scroll_wire(peer_id, block.block)
            }
            NewBlockEvent::Hashes(_) => {
                warn!(target: "scroll::bridge::import", peer_id = %peer_id, "Received NewBlockHashes event, expected NewBlock event");
            }
        }
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<reth_network::import::BlockImportEvent<reth_scroll_primitives::ScrollBlock>> {
        Poll::Pending
    }
}
