use reth_network::{import::BlockImport, NetworkPrimitives};
use reth_network_peers::PeerId;
use scroll_wire::Event;
use secp256k1::{Message, SecretKey};
use tokio::sync::mpsc::UnboundedSender;

/// A block import implementation that uses the ScrollWire protocol to import blocks.
#[derive(Debug)]
pub struct GossipBlockImport {
    events: UnboundedSender<Event>,
    secret_key: SecretKey,
}

impl GossipBlockImport {
    /// Creates a new [`GossipBlockImport`] instance.
    pub fn new(events: UnboundedSender<Event>, secret_key: SecretKey) -> Self {
        Self { events, secret_key }
    }
}

impl BlockImport for GossipBlockImport {
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
        // TODO: Instead of signing the block we should extract the signature from the extra data
        // of the block.
        println!("new block from peer: {:?}", peer_id);
        let hash = incoming_block.block.block.hash_slow();
        let signature = self.secret_key.sign_ecdsa(Message::from_digest(*hash));

        // We trigger a new block event to be sent to the network manager. If this results in an error
        // it means the network manager has been dropped.
        let _ = self.events.send(Event::NewBlock {
            peer_id,
            block: incoming_block.block.block.clone(),
            signature,
        });
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
