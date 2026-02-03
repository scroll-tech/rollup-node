use crate::{BlockImportError, BlockValidationError};

use super::{
    BlockImportOutcome, BlockValidation, NetworkHandleMessage, NewBlockWithPeer,
    ScrollNetworkHandle, ScrollNetworkManagerEvent,
};
use alloy_primitives::{Address, Signature, B256, U128};
use futures::StreamExt;
use reth_chainspec::EthChainSpec;
use reth_eth_wire_types::NewBlock as EthWireNewBlock;
use reth_network::{
    cache::LruCache, NetworkConfig as RethNetworkConfig, NetworkHandle as RethNetworkHandle,
    NetworkManager as RethNetworkManager,
};
use reth_network_api::{block::NewBlockWithPeer as RethNewBlockWithPeer, FullNetwork, PeerId};
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
use std::{collections::HashMap, sync::Arc};
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
    /// Tracks block hashes received from each peer for duplicate detection.
    peer_state: HashMap<PeerId, LruCache<B256>>,
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
        let peer_state = HashMap::new();

        // Spawn the inner network manager.
        tokio::spawn(inner_network_manager);

        (
            Self {
                chain_spec,
                inner_network_handle,
                from_handle_rx: from_handle_rx.into(),
                scroll_wire,
                blocks_seen,
                peer_state,
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
        let peer_state = HashMap::new();

        (
            Self {
                chain_spec,
                inner_network_handle,
                from_handle_rx: from_handle_rx.into(),
                scroll_wire,
                blocks_seen,
                peer_state,
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

    /// Checks if a peer has already seen a specific block.
    fn peer_has_seen_block(&self, peer_id: &PeerId, block_hash: &B256) -> bool {
        self.peer_state.get(peer_id).is_some_and(|state| state.contains(block_hash))
    }

    /// Marks a block as seen by a peer, returning `true` if this is a duplicate.
    fn mark_block_seen(&mut self, peer_id: PeerId, block_hash: B256) -> bool {
        let state = self.peer_state.entry(peer_id).or_insert_with(|| LruCache::new(LRU_CACHE_SIZE));
        if state.contains(&block_hash) {
            return true;
        }
        state.insert(block_hash);
        false
    }

    /// Main execution loop for the [`ScrollNetworkManager`].
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                biased;

                // Handle messages from the network handle.
                message = self.from_handle_rx.next() => {
                    match message {
                        Some(message) => {
                            self.on_handle_message(message).await;
                        }
                        // All network handles have been dropped so we can shutdown the network.
                        None => {
                            trace!(target: "scroll::network::manager", "Network handle channel closed, shutting down network manager");
                            return;
                        }
                    }
                }

                // Handle scroll-wire events.
                event = &mut self.scroll_wire => {
                    if let Some(event) = self.on_scroll_wire_event(event) {
                        self.event_sender.notify(event);
                    }
                }

                // Handle blocks received from the eth-wire protocol.
                Some(block) = Self::next_eth_wire_block(&mut self.eth_wire_listener) => {
                    if let Some(event) = self.handle_eth_wire_block(block) {
                        self.event_sender.notify(event);
                    }
                }
            }
        }
    }

    async fn next_eth_wire_block(
        eth_wire_listener: &mut Option<
            EventStream<reth_network_api::block::NewBlockWithPeer<ScrollBlock>>,
        >,
    ) -> Option<reth_network_api::block::NewBlockWithPeer<ScrollBlock>> {
        match eth_wire_listener.as_mut() {
            Some(listener) => listener.next().await,
            None => std::future::pending().await,
        }
    }

    /// Announces a new block to the network.
    async fn announce_block(&mut self, block: NewBlock) {
        #[cfg(feature = "test-utils")]
        if !self.gossip {
            return;
        }

        let hash = block.block.hash_slow();

        let peers = match self.inner_network_handle.get_all_peers().await {
            Ok(peers) => peers,
            Err(e) => {
                tracing::error!(target: "scroll::network::manager", "Failed to get all peers: {}", e);
                return;
            }
        };

        // TODO: remove this once we deprecate l2geth.
        let should_announce_eth_wire = self.verify_block_signature(&block);

        // Lazily build eth-wire block only if needed
        let mut eth_wire_new_block = None;

        for peer in peers {
            // Skip peers that have already seen this block
            if self.peer_has_seen_block(&peer.remote_id, &hash) {
                continue;
            }

            // Announce via scroll-wire if peer supports it
            if self.scroll_wire.is_connected(peer.remote_id) {
                if let Err(e) = self.scroll_wire.announce_block(peer.remote_id, &block) {
                    trace!(target: "scroll::network::manager", peer_id = %peer.remote_id, block_number = %block.block.header.number, block_hash = %hash, error = ?e, "Failed to announce block to peer via scroll-wire");
                    self.peer_state.remove(&peer.remote_id);
                }
            } else if should_announce_eth_wire {
                // Build eth-wire block on first use
                let eth_wire_block = eth_wire_new_block.get_or_insert_with(|| {
                    let td = compute_td(self.td_constant, block.block.header.number);
                    let mut eth_wire_block = block.block.clone();
                    eth_wire_block.header.extra_data = block.signature.clone().into();
                    EthWireNewBlock { block: eth_wire_block, td }
                });

                trace!(target: "scroll::network::manager", peer_id = %peer.remote_id, block_number = %block.block.header.number, block_hash = %hash, "Announcing new block to peer via eth-wire");
                self.inner_network_handle.eth_wire_announce_block_to_peer(
                    peer.remote_id,
                    eth_wire_block.clone(),
                    hash,
                );
            }
        }
    }

    /// Verifies the block signature against the authorized signer.
    ///
    /// Returns true if the signature is valid or if no authorized signer is configured.
    fn verify_block_signature(&self, block: &NewBlock) -> bool {
        let Some(authorized_signer) = self.authorized_signer else {
            return true;
        };

        let sig_hash = sig_encode_hash(&block.block.header);
        let Ok(signature) = Signature::from_raw(&block.signature) else {
            return false;
        };
        let Ok(recovered_signer) =
            reth_primitives_traits::crypto::secp256k1::recover_signer(&signature, sig_hash)
        else {
            return false;
        };

        authorized_signer == recovered_signer
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

                // Check if we have already received this block via scroll-wire from this peer, if
                // so penalize it.
                if self.mark_block_seen(peer_id, block_hash) {
                    tracing::warn!(target: "scroll::network::manager", peer_id = ?peer_id, block = ?block_hash, "Peer sent duplicate block via scroll-wire, penalizing");
                    self.inner_network_handle.reputation_change(
                        peer_id,
                        reth_network_api::ReputationChangeKind::BadBlock,
                    );
                    return None;
                }

                if self.blocks_seen.contains(&(block_hash, signature)) {
                    None
                } else {
                    // Update the state of the block cache i.e. we have seen this block.
                    self.blocks_seen.insert((block_hash, signature));

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
    async fn on_handle_message(&mut self, message: NetworkHandleMessage) {
        match message {
            NetworkHandleMessage::AnnounceBlock { block, signature } => {
                self.announce_block(NewBlock::new(signature, block)).await
            }
            NetworkHandleMessage::BlockImportOutcome(outcome) => {
                self.on_block_import_result(outcome).await;
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
    async fn on_block_import_result(&mut self, outcome: BlockImportOutcome) {
        let BlockImportOutcome { peer, result } = outcome;
        match result {
            Ok(BlockValidation::ValidBlock { new_block: msg }) |
            Ok(BlockValidation::ValidHeader { new_block: msg }) => {
                trace!(target: "scroll::network::manager", peer_id = ?peer, block = %Into::<BlockInfo>::into(&msg.block), "Block import successful - announcing block to network");
                self.announce_block(msg).await;
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
            Err(BlockImportError::L2FinalizedBlockReceived(peer)) => {
                trace!(target: "scroll::network::manager", peer_id = ?peer, "Block import failed - finalized block received - penalizing peer");
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

            // Check if we have already received this block from this peer via eth-wire, if so,
            // penalize the peer.
            if self.mark_block_seen(peer_id, block_hash) {
                tracing::warn!(target: "scroll::bridge::import", peer_id = ?peer_id, block = ?block_hash, "Peer sent duplicate block via eth-wire, penalizing");
                self.inner_network_handle
                    .reputation_change(peer_id, reth_network_api::ReputationChangeKind::BadBlock);
                return None;
            }

            if self.blocks_seen.contains(&(block_hash, signature)) {
                None
            } else {
                trace!(target: "scroll::bridge::import", peer_id = %peer_id, block_hash = %block_hash, signature = %signature.to_string(), extra_data = %extra_data.to_string(), "Received new block from eth-wire protocol");

                // Update the state of the block cache i.e. we have seen this block.
                self.blocks_seen.insert((block_hash, signature));
                Some(ScrollNetworkManagerEvent::NewBlock(NewBlockWithPeer {
                    peer_id,
                    block,
                    signature,
                }))
            }
        } else {
            tracing::warn!(target: "scroll::bridge::import", peer_id = %peer_id, "Failed to extract signature from block extra data, penalizing peer");
            self.inner_network_handle
                .reputation_change(peer_id, reth_network_api::ReputationChangeKind::BadBlock);
            None
        }
    }
}

/// Compute totally difficulty for a given block number.
fn compute_td(td_constant: U128, block_number: u64) -> U128 {
    td_constant.saturating_add(U128::from(block_number))
}
