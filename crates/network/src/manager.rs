use crate::{BlockImportError, BlockValidationError};

use super::{
    BlockImportOutcome, BlockValidation, NetworkHandleMessage, NetworkManagerEvent,
    NewBlockWithPeer, ScrollNetworkHandle,
};
use alloy_primitives::{FixedBytes, Signature, U128};
use futures::{FutureExt, Stream, StreamExt};
use reth_chainspec::EthChainSpec;
use reth_eth_wire_types::NewBlock as EthWireNewBlock;
use reth_network::{
    cache::LruCache, NetworkConfig as RethNetworkConfig, NetworkHandle as RethNetworkHandle,
    NetworkManager as RethNetworkManager,
};
use reth_network_api::{block::NewBlockWithPeer as RethNewBlockWithPeer, FullNetwork};
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_scroll_primitives::ScrollBlock;
use reth_storage_api::BlockNumReader as BlockNumReaderT;
use reth_tokio_util::EventStream;
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_wire::{
    NewBlock, ScrollWireConfig, ScrollWireEvent, ScrollWireManager, ScrollWireProtocolHandler,
    LRU_CACHE_SIZE,
};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::trace;

/// The size of the ECDSA signature in bytes.
const ECDSA_SIGNATURE_LEN: usize = 65;

/// [`ScrollNetworkManager`] manages the state of the scroll p2p network.
///
/// This manager drives the state of the entire network forward and includes the following
/// components:
/// - `handle`: Used to interact with this [`ScrollNetworkManager`] by sending messages using the
///   [`FullNetwork`].
/// - `from_handle_rx`: Receives commands from the [`FullNetwork`].
/// - `scroll_wire`: The type that manages connections and state of the scroll wire protocol.
#[derive(Debug)]
pub struct ScrollNetworkManager<N, CS> {
    /// The chain spec used by the rollup node.
    chain_spec: Arc<CS>,
    /// A handle used to interact with the network manager.
    handle: ScrollNetworkHandle<N>,
    /// Receiver half of the channel set up between this type and the [`FullNetwork`], receives
    /// [`NetworkHandleMessage`]s.
    from_handle_rx: UnboundedReceiverStream<NetworkHandleMessage>,
    /// The receiver for new blocks received from the network (used to bridge from eth-wire).
    eth_wire_listener: Option<EventStream<RethNewBlockWithPeer<ScrollBlock>>>,
    /// The scroll wire protocol manager.
    scroll_wire: ScrollWireManager,
    /// Should blocks be announced over the eth-wire protocol.
    eth_wire_gossip: bool,
}

impl<CS: ScrollHardforks + EthChainSpec + Send + Sync + 'static>
    ScrollNetworkManager<RethNetworkHandle<ScrollNetworkPrimitives>, CS>
{
    /// Creates a new [`ScrollNetworkManager`] instance from the provided configuration and block
    /// import.
    pub async fn new<C: BlockNumReaderT + 'static>(
        chain_spec: Arc<CS>,
        mut network_config: RethNetworkConfig<C, ScrollNetworkPrimitives>,
        scroll_wire_config: ScrollWireConfig,
        eth_wire_listener: Option<EventStream<RethNewBlockWithPeer<ScrollBlock>>>,
        eth_wire_gossip: bool,
    ) -> Self {
        // Create the scroll-wire protocol handler.
        let (scroll_wire_handler, events) = ScrollWireProtocolHandler::new(scroll_wire_config);

        // Add the scroll-wire protocol to the network configuration.
        network_config.extra_protocols.push(scroll_wire_handler);

        // Create the inner network manager.
        let inner_network_manager =
            RethNetworkManager::<ScrollNetworkPrimitives>::new(network_config).await.unwrap();
        let inner_network_handle = inner_network_manager.handle().clone();

        // Create the channel for sending messages to the network manager.
        let (to_manager_tx, from_handle_rx) = mpsc::unbounded_channel();

        let handle = ScrollNetworkHandle::new(to_manager_tx, inner_network_handle);

        // Create the scroll-wire protocol manager.
        let scroll_wire = ScrollWireManager::new(events);

        // Spawn the inner network manager.
        tokio::spawn(inner_network_manager);

        Self {
            chain_spec,
            handle,
            from_handle_rx: from_handle_rx.into(),
            scroll_wire,
            eth_wire_listener,
            eth_wire_gossip,
        }
    }
}

impl<
        N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
        CS: ScrollHardforks + EthChainSpec + Send + Sync + 'static,
    > ScrollNetworkManager<N, CS>
{
    /// Creates a new [`ScrollNetworkManager`] instance from the provided parts.
    ///
    /// This is used when the scroll-wire [`ScrollWireProtocolHandler`] and the inner network
    /// manager [`RethNetworkManager`] are instantiated externally.
    pub fn from_parts(
        chain_spec: Arc<CS>,
        inner_network_handle: N,
        events: UnboundedReceiver<ScrollWireEvent>,
        eth_wire_listener: Option<EventStream<RethNewBlockWithPeer<ScrollBlock>>>,
        eth_wire_gossip: bool,
    ) -> Self {
        // Create the channel for sending messages to the network manager from the network handle.
        let (to_manager_tx, from_handle_rx) = mpsc::unbounded_channel();

        // Create the scroll-wire protocol manager.
        let scroll_wire = ScrollWireManager::new(events);

        let handle = ScrollNetworkHandle::new(to_manager_tx, inner_network_handle);

        Self {
            chain_spec,
            handle,
            from_handle_rx: from_handle_rx.into(),
            scroll_wire,
            eth_wire_listener,
            eth_wire_gossip,
        }
    }

    /// Returns a new [`ScrollNetworkHandle`] instance.
    pub fn handle(&self) -> &ScrollNetworkHandle<N> {
        &self.handle
    }

    /// Returns an inner network handle [`RethNetworkHandle`].
    pub fn inner_network_handle(&self) -> &N {
        self.handle.inner()
    }

    /// Announces a new block to the network.
    fn announce_block(&mut self, block: NewBlock) {
        // Compute the block hash.
        let hash = block.block.hash_slow();

        // Filter the peers that have not seen this block hash.
        let peers: Vec<FixedBytes<64>> = self
            .scroll_wire
            .state()
            .iter()
            .filter_map(|(peer_id, blocks)| (!blocks.contains(&hash)).then_some(*peer_id))
            .collect();

        if self.eth_wire_gossip {
            let eth_wire_new_block = {
                let td = U128::from_limbs([0, block.block.header.number]);
                let mut eth_wire_block = block.block.clone();
                eth_wire_block.header.extra_data = block.signature.clone().into();
                EthWireNewBlock { block: eth_wire_block, td }
            };
            self.inner_network_handle().eth_wire_announce_block(eth_wire_new_block, hash);
        }

        // Announce block to the filtered set of peers
        for peer_id in peers {
            trace!(target: "scroll::network::manager", peer_id = %peer_id, block_hash = %hash, "Announcing new block to peer");
            self.scroll_wire.announce_block(peer_id, &block, hash);
        }
    }

    /// Handler for received events from the [`ScrollWireManager`].
    fn on_scroll_wire_event(&mut self, event: ScrollWireEvent) -> NetworkManagerEvent {
        match event {
            ScrollWireEvent::NewBlock { peer_id, block, signature } => {
                trace!(target: "scroll::network::manager", peer_id = ?peer_id, block = ?block.hash_slow(), signature = ?signature, "Received new block");
                NetworkManagerEvent::NewBlock(NewBlockWithPeer { peer_id, block, signature })
            }
            // Only `NewBlock` events are expected from the scroll-wire protocol.
            _ => {
                unreachable!()
            }
        }
    }

    /// Handler for received messages from the [`ScrollNetworkHandle`].
    fn on_handle_message(&mut self, message: NetworkHandleMessage) {
        match message {
            NetworkHandleMessage::AnnounceBlock { block, signature } => {
                self.announce_block(NewBlock::new(signature, block))
            }
            NetworkHandleMessage::BlockImportOutcome(outcome) => {
                self.on_block_import_result(outcome);
            }
            NetworkHandleMessage::Shutdown(tx) => {
                tx.send(()).unwrap()
                // self.perform_network_shutdown().await;
                // let _ = tx.send(());
            }
        }
    }

    /// Handler for the result of a block import.
    fn on_block_import_result(&mut self, outcome: BlockImportOutcome) {
        let BlockImportOutcome { peer, result } = outcome;
        match result {
            Ok(BlockValidation::ValidBlock { new_block: msg }) |
            Ok(BlockValidation::ValidHeader { new_block: msg }) => {
                trace!(target: "scroll::network::manager", peer_id = ?peer, block = ?msg.block, "Block import successful - announcing block to network");
                let hash = msg.block.hash_slow();
                self.scroll_wire
                    .state_mut()
                    .entry(peer)
                    .or_insert_with(|| LruCache::new(LRU_CACHE_SIZE))
                    .insert(hash);
                self.announce_block(msg);
            }
            Err(BlockImportError::Consensus(err)) => {
                trace!(target: "scroll::network::manager", peer_id = ?peer, ?err, "Block import failed - consensus error - penalizing peer");
                self.inner_network_handle()
                    .reputation_change(peer, reth_network_api::ReputationChangeKind::BadBlock);
            }
            Err(BlockImportError::Validation(BlockValidationError::InvalidBlock)) => {
                trace!(target: "scroll::network::manager", peer_id = ?peer, "Block import failed - invalid block - penalizing peer");
                self.inner_network_handle()
                    .reputation_change(peer, reth_network_api::ReputationChangeKind::BadBlock);
            }
        }
    }

    /// Handles a new block received from the eth-wire protocol.
    fn handle_eth_wire_block(
        &mut self,
        block: reth_network_api::block::NewBlockWithPeer<ScrollBlock>,
    ) -> Option<NetworkManagerEvent> {
        let reth_network_api::block::NewBlockWithPeer { peer_id, mut block } = block;
        let block_hash = block.hash_slow();
        self.scroll_wire
            .state_mut()
            .entry(peer_id)
            .or_insert_with(|| LruCache::new(LRU_CACHE_SIZE))
            .insert(block_hash);

        trace!(target: "scroll::bridge::import", peer_id = %peer_id, block_hash = %block_hash, "Received new block from eth-wire protocol");

        // We purge the extra data field post euclid v2 to align with protocol specification.
        let extra_data = if self.chain_spec.is_euclid_v2_active_at_timestamp(block.timestamp) {
            let extra_data = block.extra_data.clone();
            block.header.extra_data = Default::default();
            extra_data
        } else {
            block.extra_data.clone()
        };

        // If we can extract a signature from the extra data we validate consensus and then attempt
        // import via the EngineAPI in the `handle_new_block` method. The signature is extracted
        // from the last `ECDSA_SIGNATURE_LEN` bytes of the extra data field as specified by
        // the protocol.
        if let Some(signature) = extra_data
            .len()
            .checked_sub(ECDSA_SIGNATURE_LEN)
            .and_then(|i| Signature::from_raw(&extra_data[i..]).ok())
        {
            Some(NetworkManagerEvent::NewBlock(NewBlockWithPeer { peer_id, block, signature }))
        } else {
            tracing::warn!(target: "scroll::bridge::import", peer_id = %peer_id, "Failed to extract signature from block extra data");
            None
        }
    }
}

impl<
        N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
        CS: ScrollHardforks + EthChainSpec + Send + Sync + 'static,
    > Stream for ScrollNetworkManager<N, CS>
{
    type Item = NetworkManagerEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // We handle the messages from the network handle.
        loop {
            match this.from_handle_rx.poll_next_unpin(cx) {
                // A message has been received from the network handle.
                Poll::Ready(Some(message)) => {
                    this.on_handle_message(message);
                }
                // All network handles have been dropped so we can shutdown the network.
                Poll::Ready(None) => {
                    return Poll::Ready(None);
                }
                // No additional messages exist break.
                Poll::Pending => break,
            }
        }

        // Next we handle the scroll-wire events.
        if let Poll::Ready(event) = this.scroll_wire.poll_unpin(cx) {
            return Poll::Ready(Some(this.on_scroll_wire_event(event)));
        }

        // Handle blocks received from the eth-wire protocol.
        while let Some(Poll::Ready(Some(block))) =
            this.eth_wire_listener.as_mut().map(|new_block_rx| new_block_rx.poll_next_unpin(cx))
        {
            if let Some(event) = this.handle_eth_wire_block(block) {
                return Poll::Ready(Some(event));
            }
        }

        Poll::Pending
    }
}
