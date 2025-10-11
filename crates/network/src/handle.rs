use super::ScrollNetwork;
use crate::ScrollNetworkManagerEvent;
use alloy_primitives::Signature;
use reth_network_api::FullNetwork;
use reth_scroll_primitives::ScrollBlock;
use reth_tokio_util::EventStream;
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

/// A handle used to communicate with the [`super::ScrollNetworkManager`].
#[derive(Debug, Clone)]
pub struct ScrollNetworkHandle<N> {
    /// A reference to the inner network handle.
    pub(crate) inner: Arc<NetworkInner<N>>,
}

impl<N: FullNetwork> ScrollNetworkHandle<N> {
    /// Creates a new [`ScrollNetworkHandle`] instance.
    pub fn new(
        to_manager_tx: UnboundedSender<NetworkHandleMessage>,
        inner_network_handle: N,
    ) -> Self {
        let inner = NetworkInner { to_manager_tx, inner_network_handle };
        Self { inner: Arc::new(inner) }
    }

    pub async fn into_scroll_network(self) -> ScrollNetwork<N> {
        let events = self.event_listener().await;
        ScrollNetwork { handle: self, events }
    }
}

/// The inner state of the [`ScrollNetworkHandle`].
#[derive(Debug)]
pub struct NetworkInner<N> {
    /// The sender half of the channel set up between this type and the
    /// [`super::ScrollNetworkManager`].
    pub(crate) to_manager_tx: UnboundedSender<NetworkHandleMessage>,
    /// Inner network handle which is used to communicate with the inner network.
    pub inner_network_handle: N,
}

impl<N: FullNetwork> ScrollNetworkHandle<N> {
    /// Returns a reference to the inner network handle.
    pub fn inner(&self) -> &N {
        &self.inner.inner_network_handle
    }

    /// Sends a message to the network manager.
    pub fn send_message(&self, msg: NetworkHandleMessage) {
        let _ = self.inner.to_manager_tx.send(msg);
    }

    pub fn block_import_outcome(&self, outcome: super::BlockImportOutcome) {
        self.send_message(NetworkHandleMessage::BlockImportOutcome(outcome));
    }

    /// Announces a block to the network.
    pub fn announce_block(&self, block: ScrollBlock, signature: Signature) {
        self.send_message(NetworkHandleMessage::AnnounceBlock { block, signature });
    }

    /// Shuts down the network handle.
    pub async fn shutdown(&self) -> Result<(), oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_message(NetworkHandleMessage::Shutdown(tx));
        rx.await
    }

    pub fn local_node_record(&self) -> reth_network_peers::NodeRecord {
        self.inner.inner_network_handle.local_node_record()
    }

    pub async fn event_listener(&self) -> EventStream<ScrollNetworkManagerEvent> {
        let (tx, rx) = oneshot::channel();
        self.send_message(NetworkHandleMessage::EventListener(tx));
        rx.await.expect("network manager dropped")
    }

    #[cfg(feature = "test-utils")]
    pub async fn set_gossip(&self, enabled: bool) {
        let (tx, rx) = oneshot::channel();
        self.send_message(NetworkHandleMessage::SetGossip((enabled, tx)));
        rx.await.expect("network manager dropped");
    }
}

/// A message type used for communication between the [`ScrollNetworkHandle`] and the
/// [`super::ScrollNetworkManager`].
#[derive(Debug)]
pub enum NetworkHandleMessage {
    AnnounceBlock {
        block: ScrollBlock,
        signature: Signature,
    },
    BlockImportOutcome(super::BlockImportOutcome),
    Shutdown(oneshot::Sender<()>),
    EventListener(oneshot::Sender<EventStream<ScrollNetworkManagerEvent>>),
    #[cfg(feature = "test-utils")]
    SetGossip((bool, oneshot::Sender<()>)),
}
