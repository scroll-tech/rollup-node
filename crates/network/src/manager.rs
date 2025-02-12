use crate::{BlockImportError, BlockValidationError};

use super::{
    BlockImportOutcome, BlockValidation, NetworkHandle, NetworkHandleMessage, NetworkManagerEvent,
    NewBlockWithPeer,
};
use alloy_primitives::FixedBytes;
use core::task::Poll;
use futures::{FutureExt, StreamExt};
use reth_network::{
    cache::LruCache, NetworkConfig as RethNetworkConfig, NetworkHandle as RethNetworkHandle,
    NetworkManager as RethNetworkManager, Peers,
};
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_storage_api::BlockNumReader as BlockNumReaderT;
use scroll_wire::{
    NewBlock, ProtocolHandler, ScrollWireConfig, ScrollWireEvent, ScrollWireManager, LRU_CACHE_SIZE,
};
use std::future::Future;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::trace;

/// [`NetworkManager`] manages the state of the scroll p2p network.
///
/// This is an endless [`Future`] that drives the state of the entire network forward and includes
/// the following components:
/// - `handle`: Used to interact with this [`NetworkManager`] by sending messages using the
///   [`NetworkHandle`].
/// - `from_handle_rx`: Receives commands from the [`NetworkHandle`].
/// - `scroll_wire`: The type that manages connections and state of the scroll wire protocol.
#[derive(Debug)]
pub struct NetworkManager {
    /// A handle used to interact with the network manager.
    handle: NetworkHandle,
    /// Receiver half of the channel set up between this type and the [`NetworkHandle`], receives
    /// [`NetworkHandleMessage`]s.
    from_handle_rx: UnboundedReceiverStream<NetworkHandleMessage>,
    /// The scroll wire protocol manager.
    scroll_wire: ScrollWireManager,
}

impl NetworkManager {
    /// Creates a new [`NetworkManager`] instance from the provided parts.
    ///
    /// This is used when the scroll-wire [`ProtocolHandler`] and the inner network manager
    /// [`RethNetworkManager`] are instantiated externally.
    pub fn from_parts(
        inner_network_handle: RethNetworkHandle<ScrollNetworkPrimitives>,
        events: UnboundedReceiver<ScrollWireEvent>,
    ) -> Self {
        // Create the channel for sending messages to the network manager from the network handle.
        let (to_manager_tx, from_handle_rx) = mpsc::unbounded_channel();

        // Create the scroll-wire protocol manager.
        let scroll_wire = ScrollWireManager::new(events);

        let handle = NetworkHandle::new(to_manager_tx, inner_network_handle);

        Self { handle, from_handle_rx: from_handle_rx.into(), scroll_wire }
    }

    /// Creates a new [`NetworkManager`] instance from the provided configuration and block import.
    pub async fn new<C: BlockNumReaderT + 'static>(
        mut network_config: RethNetworkConfig<C, ScrollNetworkPrimitives>,
        scroll_wire_config: ScrollWireConfig,
    ) -> Self {
        // Create the scroll-wire protocol handler.
        let (scroll_wire_handler, events) = ProtocolHandler::new(scroll_wire_config);

        // Add the scroll-wire protocol to the network configuration.
        network_config.extra_protocols.push(scroll_wire_handler);

        // Create the inner network manager.
        let inner_network_manager =
            RethNetworkManager::<ScrollNetworkPrimitives>::new(network_config).await.unwrap();
        let inner_network_handle = inner_network_manager.handle().clone();

        // Create the channel for sending messages to the network manager.
        let (to_manager_tx, from_handle_rx) = mpsc::unbounded_channel();

        let handle = NetworkHandle::new(to_manager_tx, inner_network_handle);

        // Create the scroll-wire protocol manager.
        let scroll_wire = ScrollWireManager::new(events);

        // Spawn the inner network manager.
        tokio::spawn(inner_network_manager);

        Self { handle, from_handle_rx: from_handle_rx.into(), scroll_wire }
    }

    /// Returns a new [`NetworkHandle`] instance.
    pub fn handle(&self) -> &NetworkHandle {
        &self.handle
    }

    /// Returns an inner network handle [`RethNetworkHandle`].
    pub fn inner_network_handle(&self) -> &RethNetworkHandle<ScrollNetworkPrimitives> {
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

        // Announce block to the filtered set of peers
        for peer_id in peers {
            trace!(target: "scroll::network::manager", peer_id = %peer_id, block_hash = %hash, "Announcing new block to peer");
            self.scroll_wire.announce_block(peer_id, &block, hash);
        }
    }

    /// Handler for received events from the [`ScrollWireManager`].
    fn on_scroll_wire_event(&mut self, event: ScrollWireEvent) -> Option<NetworkManagerEvent> {
        match event {
            ScrollWireEvent::NewBlock { peer_id, block, signature } => {
                trace!(target: "scroll::network::manager", peer_id = ?peer_id, block = ?block, signature = ?signature, "Received new block");
                Some(NetworkManagerEvent::NewBlock(NewBlockWithPeer { peer_id, block, signature }))
            }
            // Only `NewBlock` events are expected from the scroll-wire protocol.
            _ => {
                unreachable!()
            }
        }
    }

    /// Handler for received messaged from the [`NetworkHandle`].
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

    pub async fn perform_network_shutdown(&mut self) {
        self.inner_network_handle().shutdown().await.unwrap()
    }
}

impl Future for NetworkManager {
    type Output = NetworkManagerEvent;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        trace!(target: "scroll::network::manager", "Polling network manager");

        let this = self.get_mut();

        // We handle the messages from the network handle.
        loop {
            match this.from_handle_rx.poll_next_unpin(cx) {
                // A message has been received from the network handle.
                std::task::Poll::Ready(Some(message)) => {
                    this.on_handle_message(message);
                }
                // All network handles have been dropped so we can shutdown the network.
                std::task::Poll::Ready(None) => {
                    // return std::task::Poll::Ready(());
                    // For now we will return pending to keep the network running.
                    break;
                }
                // No additional messages exist break.
                std::task::Poll::Pending => break,
            }
        }

        // Next we handle the scroll-wire events.
        while let Poll::Ready(event) = this.scroll_wire.poll_unpin(cx) {
            if let Some(event) = this.on_scroll_wire_event(event) {
                return std::task::Poll::Ready(event);
            }
        }

        std::task::Poll::Pending
    }
}
