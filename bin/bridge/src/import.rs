use reth_network::{import::BlockImport, NetworkPrimitives};
use reth_network_peers::PeerId;
use scroll_wire::Event;
use secp256k1::ecdsa::Signature;
use tokio::sync::mpsc::UnboundedSender;
use tracing::trace;

const ECDSA_SIGNATURE_LEN: usize = 64;

/// A block import implementation that uses the ScrollWire protocol to import blocks.
#[derive(Debug)]
pub struct BridgeBlockImport {
    events: UnboundedSender<Event>,
}

impl BridgeBlockImport {
    /// Creates a new [`GossipBlockImport`] instance.
    pub fn new(events: UnboundedSender<Event>) -> Self {
        Self { events }
    }
}

impl BlockImport for BridgeBlockImport {
    /// This function is called when a new block is received from the network.
    ///
    /// It signs the block with the secret key and sends a new block event to the network manager.
    /// The network manager will then attempt to import the block and broadcast it to the network.
    fn on_new_block(
        &mut self,
        peer_id: PeerId,
        incoming_block: reth_network::message::NewBlockMessage<
            <reth_network::EthNetworkPrimitives as NetworkPrimitives>::Block,
        >,
    ) {
        let extra_data = &incoming_block.block.block.extra_data;
        let signature =
            Signature::from_compact(&extra_data[extra_data.len() - ECDSA_SIGNATURE_LEN..])
                .expect("Failed to parse signature");
        let block = incoming_block.block.block.clone();

        trace!(target: "bridge::import", peer_id = %peer_id, block = ?block, "Received new block from eth-wire protocol");

        // We trigger a new block event to be sent to the network manager. If this results in an
        // error it means the network manager has been dropped.
        let _ = self.events.send(Event::NewBlock { peer_id, block, signature });
    }

    /// There is no polling required for the gossip block import type.
    fn poll(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<
        reth_network::import::BlockImportOutcome<
            <reth_network::EthNetworkPrimitives as NetworkPrimitives>::Block,
        >,
    > {
        std::task::Poll::Pending
    }
}
