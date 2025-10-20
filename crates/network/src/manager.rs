use crate::{BlockImportError, BlockValidationError};

use super::{
    BlockImportOutcome, BlockValidation, NetworkHandleMessage, NewBlockWithPeer,
    ScrollNetworkHandle, ScrollNetworkManagerEvent,
};
use alloy_primitives::{Address, FixedBytes, Signature, B256, U128};
use futures::{Future, FutureExt, StreamExt};
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
use reth_tokio_util::{EventSender, EventStream};
use rollup_node_primitives::{sig_encode_hash, BlockInfo};
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

const EVENT_CHANNEL_SIZE: usize = 5000;

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
    /// The inner network handle which is used to communicate with the inner network.
    inner_network_handle: N,
    /// Receiver half of the channel set up between this type and the [`FullNetwork`], receives
    /// [`NetworkHandleMessage`]s.
    from_handle_rx: UnboundedReceiverStream<NetworkHandleMessage>,
    /// The receiver for new blocks received from the network (used to bridge from eth-wire).
    eth_wire_listener: Option<EventStream<RethNewBlockWithPeer<ScrollBlock>>>,
    /// The scroll wire protocol manager.
    pub scroll_wire: ScrollWireManager,
    /// The LRU cache used to track already seen (block,signature) pair.
    pub blocks_seen: LruCache<(B256, Signature)>,
    /// The constant value that must be added to the block number to get the total difficulty.
    td_constant: U128,
    /// The authorized signer for the network.
    authorized_signer: Option<Address>,
    /// Whether to gossip blocks to peers.
    #[cfg(feature = "test-utils")]
    gossip: bool,
    /// The event sender for network events.
    event_sender: EventSender<ScrollNetworkManagerEvent>,
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
        td_constant: U128,
        authorized_signer: Option<Address>,
    ) -> (Self, ScrollNetworkHandle<RethNetworkHandle<ScrollNetworkPrimitives>>) {
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

        let handle = ScrollNetworkHandle::new(to_manager_tx, inner_network_handle.clone());
        let event_sender = EventSender::new(EVENT_CHANNEL_SIZE);

        // Create the scroll-wire protocol manager.
        let scroll_wire = ScrollWireManager::new(events);

        let blocks_seen = LruCache::new(LRU_CACHE_SIZE);

        // Spawn the inner network manager.
        tokio::spawn(inner_network_manager);

        (
            Self {
                chain_spec,
                inner_network_handle,
                from_handle_rx: from_handle_rx.into(),
                scroll_wire,
                blocks_seen,
                eth_wire_listener,
                td_constant,
                authorized_signer,
                event_sender,
                #[cfg(feature = "test-utils")]
                gossip: true,
            },
            handle,
        )
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
        td_constant: U128,
        authorized_signer: Option<Address>,
    ) -> (Self, ScrollNetworkHandle<N>) {
        // Create the channel for sending messages to the network manager from the network handle.
        let (to_manager_tx, from_handle_rx) = mpsc::unbounded_channel();

        // Create the scroll-wire protocol manager.
        let scroll_wire = ScrollWireManager::new(events);

        let handle = ScrollNetworkHandle::new(to_manager_tx, inner_network_handle.clone());
        let event_sender = EventSender::new(EVENT_CHANNEL_SIZE);

        let blocks_seen = LruCache::new(LRU_CACHE_SIZE);

        (
            Self {
                chain_spec,
                inner_network_handle,
                from_handle_rx: from_handle_rx.into(),
                scroll_wire,
                blocks_seen,
                eth_wire_listener,
                td_constant,
                authorized_signer,
                event_sender,
                #[cfg(feature = "test-utils")]
                gossip: true,
            },
            handle,
        )
    }

    /// Announces a new block to the network.
    fn announce_block(&mut self, block: NewBlock) {
        #[cfg(feature = "test-utils")]
        if !self.gossip {
            return;
        }

        // Compute the block hash.
        let hash = block.block.hash_slow();

        // Filter the peers that have not seen this block hash.
        let peers: Vec<FixedBytes<64>> = self
            .scroll_wire
            .state()
            .iter()
            .filter_map(|(peer_id, blocks)| (!blocks.contains(&hash)).then_some(*peer_id))
            .collect();

        // TODO: remove this once we deprecate l2geth.
        // Determine if we should announce via eth wire
        let should_announce_eth_wire = if let Some(authorized_signer) = self.authorized_signer {
            // Only announce if the block signature matches the authorized signer
            let sig_hash = sig_encode_hash(&block.block.header);
            if let Ok(signature) = Signature::from_raw(&block.signature) {
                if let Ok(recovered_signer) =
                    reth_primitives_traits::crypto::secp256k1::recover_signer(&signature, sig_hash)
                {
                    authorized_signer == recovered_signer
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            // If no authorized signer is set, always announce
            true
        };

        // Announce via eth wire if allowed
        if should_announce_eth_wire {
            let eth_wire_new_block = {
                let td = compute_td(self.td_constant, block.block.header.number);
                let mut eth_wire_block = block.block.clone();
                eth_wire_block.header.extra_data = block.signature.clone().into();
                EthWireNewBlock { block: eth_wire_block, td }
            };
            self.inner_network_handle.eth_wire_announce_block(eth_wire_new_block, hash);
        }

        // Announce block to the filtered set of peers
        for peer_id in peers {
            trace!(target: "scroll::network::manager", peer_id = %peer_id, block_number = %block.block.header.number, block_hash = %hash, "Announcing new block to peer");
            self.scroll_wire.announce_block(peer_id, &block, hash);
        }
    }

    /// Handler for received events from the [`ScrollWireManager`].
    fn on_scroll_wire_event(
        &mut self,
        event: ScrollWireEvent,
    ) -> Option<ScrollNetworkManagerEvent> {
        match event {
            ScrollWireEvent::NewBlock { peer_id, block, signature } => {
                let block_hash = block.hash_slow();
                trace!(target: "scroll::network::manager", peer_id = ?peer_id, block = ?block_hash, signature = ?signature, "Received new block");
                if self.blocks_seen.contains(&(block_hash, signature)) {
                    None
                } else {
                    // Update the state of the peer cache i.e. peer has seen this block.
                    self.scroll_wire
                        .state_mut()
                        .entry(peer_id)
                        .or_insert_with(|| LruCache::new(LRU_CACHE_SIZE))
                        .insert(block_hash);
                    // Update the state of the block cache i.e. we have seen this block.
                    self.blocks_seen.insert((block.hash_slow(), signature));

                    Some(ScrollNetworkManagerEvent::NewBlock(NewBlockWithPeer {
                        peer_id,
                        block,
                        signature,
                    }))
                }
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
            NetworkHandleMessage::EventListener(tx) => {
                let _ = tx.send(self.event_sender.new_listener());
            }
            #[cfg(feature = "test-utils")]
            NetworkHandleMessage::SetGossip((enabled, tx)) => {
                self.gossip = enabled;
                let _ = tx.send(());
            }
        }
    }

    /// Handler for the result of a block import.
    fn on_block_import_result(&mut self, outcome: BlockImportOutcome) {
        let BlockImportOutcome { peer, result } = outcome;
        match result {
            Ok(BlockValidation::ValidBlock { new_block: msg }) |
            Ok(BlockValidation::ValidHeader { new_block: msg }) => {
                trace!(target: "scroll::network::manager", peer_id = ?peer, block = %Into::<BlockInfo>::into(&msg.block), "Block import successful - announcing block to network");
                self.announce_block(msg);
            }
            Err(BlockImportError::Consensus(err)) => {
                trace!(target: "scroll::network::manager", peer_id = ?peer, ?err, "Block import failed - consensus error - penalizing peer");
                self.inner_network_handle
                    .reputation_change(peer, reth_network_api::ReputationChangeKind::BadBlock);
            }
            Err(BlockImportError::Validation(BlockValidationError::InvalidBlock)) => {
                trace!(target: "scroll::network::manager", peer_id = ?peer, "Block import failed - invalid block - penalizing peer");
                self.inner_network_handle
                    .reputation_change(peer, reth_network_api::ReputationChangeKind::BadBlock);
            }
        }
    }

    /// Handles a new block received from the eth-wire protocol.
    fn handle_eth_wire_block(
        &mut self,
        block: reth_network_api::block::NewBlockWithPeer<ScrollBlock>,
    ) -> Option<ScrollNetworkManagerEvent> {
        let reth_network_api::block::NewBlockWithPeer { peer_id, mut block } = block;

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
            let block_hash = block.hash_slow();
            if self.blocks_seen.contains(&(block_hash, signature)) {
                return None;
            }
            trace!(target: "scroll::bridge::import", peer_id = %peer_id, block_hash = %block_hash, signature = %signature.to_string(), extra_data = %extra_data.to_string(), "Received new block from eth-wire protocol");

            // Update the state of the peer cache i.e. peer has seen this block.
            self.scroll_wire
                .state_mut()
                .entry(peer_id)
                .or_insert_with(|| LruCache::new(LRU_CACHE_SIZE))
                .insert(block_hash);

            // Update the state of the block cache i.e. we have seen this block.
            self.blocks_seen.insert((block_hash, signature));
            Some(ScrollNetworkManagerEvent::NewBlock(NewBlockWithPeer {
                peer_id,
                block,
                signature,
            }))
        } else {
            tracing::warn!(target: "scroll::bridge::import", peer_id = %peer_id, "Failed to extract signature from block extra data, penalizing peer");
            self.inner_network_handle
                .reputation_change(peer_id, reth_network_api::ReputationChangeKind::BadBlock);
            None
        }
    }
}

impl<
        N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
        CS: ScrollHardforks + EthChainSpec + Send + Sync + 'static,
    > Future for ScrollNetworkManager<N, CS>
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
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
                    return Poll::Ready(());
                }
                // No additional messages exist break.
                Poll::Pending => break,
            }
        }

        // Next we handle the scroll-wire events.
        while let Poll::Ready(event) = this.scroll_wire.poll_unpin(cx) {
            if let Some(event) = this.on_scroll_wire_event(event) {
                this.event_sender.notify(event);
            }
        }

        // Handle blocks received from the eth-wire protocol.
        while let Some(Poll::Ready(Some(block))) =
            this.eth_wire_listener.as_mut().map(|new_block_rx| new_block_rx.poll_next_unpin(cx))
        {
            if let Some(event) = this.handle_eth_wire_block(block) {
                this.event_sender.notify(event);
            }
        }

        Poll::Pending
    }
}

/// Compute totally difficulty for a given block number.
fn compute_td(td_constant: U128, block_number: u64) -> U128 {
    td_constant.saturating_add(U128::from(block_number))
}
