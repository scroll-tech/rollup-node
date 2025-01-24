use reth_network::{import::BlockImport, NetworkPrimitives};
use reth_network_peers::PeerId;
use scroll_wire::Event;
use secp256k1::ecdsa::Signature;
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
    inner: Box<dyn BlockImport>,
}

impl BridgeBlockImport {
    /// Creates a new [`BridgeBlockImport`] instance with the provided events sender and inner block
    /// import.
    pub fn new(events: UnboundedSender<Event>, inner_block_import: Box<dyn BlockImport>) -> Self {
        Self { events, inner: inner_block_import }
    }
}

impl BlockImport for BridgeBlockImport {
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
        // protocol.
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
