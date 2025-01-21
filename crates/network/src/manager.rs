use super::{
    BlockImport, BlockImportOutcome, BlockValidation, NetworkHandle, NetworkHandleMessage,
};
use alloy_primitives::FixedBytes;
use core::task::Poll;
use futures::{FutureExt, StreamExt};
use reth_network::Peers;
use reth_network::{
    cache::LruCache, NetworkConfig as RethNetworkConfig, NetworkHandle as RethNetworkHandle,
    NetworkManager as RethNetworkManager,
};
use reth_storage_api::BlockNumReader as BlockNumReaderT;
use scroll_wire::{Event, NewBlock, ScrollWireConfig};
use scroll_wire::{ProtocolHandler, ScrollWireManager};
use std::future::Future;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;

/// Manages the state of the rollup node network.
///
/// This is an endless [`Future`] that drives the state of the entire network forward and includes
/// the following components:
/// - inner_network_handle: Responsible for discovery peers and managing connections on the network.
/// - block_import: Responsible for importing blocks that have been gossiped over the network.
/// - to_manager_tx: Used to interact with this [`NetworkManager`] by sending messages using the
///   [`NetworkHandle`].
/// - from_handle_rx: Receives commands from the [`NetworkHandle`].
/// - scroll_wire: The scroll-wire protocol that manages connections and state of the scroll-wire
///   protocol.
pub struct NetworkManager {
    /// A handle to the inner reth network manager.
    inner_network_handle: RethNetworkHandle,
    /// Handles block imports for new blocks received from the network.
    block_import: Box<dyn BlockImport>,
    /// The receiver half of the channel set up between this type and the [`NetworkHandle`], receives
    /// commands from the [`NetworkHandle`].
    to_manager_tx: UnboundedSender<NetworkHandleMessage>,
    /// Receiver half of the channel set up between this type and the [`NetworkHandle`], receives
    /// [`NetworkHandleMessage`]s.
    from_handle_rx: UnboundedReceiverStream<NetworkHandleMessage>,
    /// The scroll-wire protocol.
    scroll_wire: ScrollWireManager,
    /// An optional source of new blocks from the eth-wire protocol.
    eth_wire_block_source: Option<UnboundedReceiverStream<Event>>,
}

impl NetworkManager {
    /// Creates a new [`NetworkManager`] instance.
    pub async fn new<C: BlockNumReaderT + 'static>(
        mut network_config: RethNetworkConfig<C, reth_network::EthNetworkPrimitives>,
        scroll_wire_config: ScrollWireConfig,
        block_import: impl BlockImport + 'static,
    ) -> Self {
        // Create the scroll-wire protocol handler.
        let (scroll_wire_handler, events) = ProtocolHandler::new(scroll_wire_config);

        // Add the scroll-wire protocol to the network configuration.
        network_config.extra_protocols.push(scroll_wire_handler);

        // Create the inner network manager.
        let inner_network = RethNetworkManager::new(network_config).await.unwrap();
        let inner_network_handle = inner_network.handle().clone();

        // Create the channel for sending messages to the network manager.
        let (to_manager_tx, from_handle_rx) = mpsc::unbounded_channel();

        // Create the scroll-wire protocol manager.
        let scroll_wire = ScrollWireManager::new(events);

        // Spawn the inner network manager.
        tokio::spawn(inner_network);

        Self {
            inner_network_handle,
            block_import: Box::new(block_import),
            to_manager_tx,
            from_handle_rx: from_handle_rx.into(),
            scroll_wire,
            eth_wire_block_source: None,
        }
    }

    /// Sets the new block source. This is used to bridge new blocks announced on the eth-wire protocol.
    pub fn with_new_block_source(mut self, source: UnboundedReceiver<Event>) -> Self {
        self.eth_wire_block_source = Some(source.into());
        self
    }

    /// Returns a new [`NetworkHandle`] instance.
    pub fn handle(&self) -> NetworkHandle {
        NetworkHandle::new(
            self.to_manager_tx.clone(),
            self.inner_network_handle.clone(),
        )
    }

    /// Announces a new block to the network.
    fn announce_block(&mut self, block: NewBlock) {
        let hash = block.block.hash_slow();

        // Filter the peers that have not seen this block hash.
        let peers: Vec<FixedBytes<64>> = self
            .scroll_wire
            .state()
            .iter()
            .filter_map(|(peer_id, blocks)| (!blocks.contains(&hash)).then_some(*peer_id))
            .collect();

        // Announced to the filtered set of peers
        for peer in peers {
            println!("Announcing block to peer {:?}", peer);
            self.scroll_wire.announce_block(peer, &block, hash);
        }
    }

    /// Handler for received events from the [`ScrollWireManager`].
    fn on_scroll_wire_event(&mut self, event: Event) {
        match event {
            Event::NewBlock {
                peer_id,
                block,
                signature,
            } => {
                self.block_import.on_new_block(peer_id, block, signature);
            }
            _ => {
                unreachable!()
            }
        }
    }

    /// Handler for received messaged from the [`NetworkHandle`].
    fn on_handle_message(&mut self, message: NetworkHandleMessage) {
        match message {
            NetworkHandleMessage::AnnounceBlock { block, signature } => {
                println!("Announcing block {signature:?} {block:?}");
                self.announce_block(NewBlock::new(signature, block))
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
            Ok(BlockValidation::ValidBlock { new_block: msg }) => {
                let hash = msg.block.hash_slow();
                self.scroll_wire
                    .state_mut()
                    .entry(peer)
                    .or_insert_with(|| LruCache::new(100))
                    .insert(hash);
                self.announce_block(msg);
            }
            Ok(BlockValidation::ValidHeader { new_block: msg }) => {
                let hash = msg.block.hash_slow();
                self.scroll_wire
                    .state_mut()
                    .entry(peer)
                    .or_insert_with(|| LruCache::new(100))
                    .insert(hash);
                self.announce_block(msg);
            }
            Err(_) => {
                self.inner_network_handle
                    .reputation_change(peer, reth_network_api::ReputationChangeKind::BadBlock);
            }
        }
    }

    pub async fn perform_network_shutdown(&mut self) {
        self.inner_network_handle.shutdown().await.unwrap()
    }
}

impl Future for NetworkManager {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let this = self.get_mut();

        println!("Polling network manager");

        // First we handle the block import results.
        while let Poll::Ready(outcome) = this.block_import.poll(cx) {
            this.on_block_import_result(outcome);
        }

        // Next we handle the scroll-wire events.
        while let Poll::Ready(event) = this.scroll_wire.poll_unpin(cx) {
            this.on_scroll_wire_event(event);
        }

        // Next we handle the messages from the eth-wire protocol.
        while let Some(Poll::Ready(Some(event))) = this
            .eth_wire_block_source
            .as_mut()
            .map(|x| x.poll_next_unpin(cx))
        {
            // we should assert that the eth-wire protocol is only sending new blocks.
            debug_assert!(matches!(event, Event::NewBlock { .. }));
            this.on_scroll_wire_event(event);
        }

        // Next we handle the messages from the network handle.
        loop {
            match this.from_handle_rx.poll_next_unpin(cx) {
                // A message has been received from the network handle.
                std::task::Poll::Ready(Some(message)) => {
                    this.on_handle_message(message);
                }
                // All network handles have been dropped so we can shutdown the network.
                std::task::Poll::Ready(None) => {
                    return std::task::Poll::Ready(());
                }
                // No additional messages exist break.
                std::task::Poll::Pending => {
                    return std::task::Poll::Pending;
                }
            }
        }
    }
}
