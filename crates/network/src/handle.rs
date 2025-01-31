use reth_network::{NetworkHandle as RethNetworkHandle, PeersInfo};
use reth_network_peers::PeerId;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_scroll_primitives::ScrollBlock;
use secp256k1::{ecdsa::Signature, SecretKey};
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

/// A handle used to communicate with the [`NetworkManager`].
#[derive(Debug, Clone)]
pub struct NetworkHandle {
    /// A reference to the inner network handle.
    pub(crate) inner: Arc<NetworkInner>,
}

impl NetworkHandle {
    /// Creates a new [`NetworkHandle`] instance from the given [`UnboundedSender`] and
    /// [`RethNetworkHandle`].
    pub fn new(
        to_manager_tx: UnboundedSender<NetworkHandleMessage>,
        inner_network_handle: RethNetworkHandle<ScrollNetworkPrimitives>,
    ) -> Self {
        let inner = NetworkInner { to_manager_tx, inner_network_handle };
        Self { inner: Arc::new(inner) }
    }
}

/// The inner state of the [`NetworkHandle`].
#[derive(Debug)]
pub struct NetworkInner {
    /// The sender half of the channel set up between this type and the [`NetworkManager`].
    pub(crate) to_manager_tx: UnboundedSender<NetworkHandleMessage>,
    /// Inner network handle which is used to communicate with the inner network.
    pub inner_network_handle: RethNetworkHandle<ScrollNetworkPrimitives>,
}

impl NetworkHandle {
    /// Returns a reference to the inner network handle.
    pub fn inner(&self) -> &RethNetworkHandle<ScrollNetworkPrimitives> {
        &self.inner.inner_network_handle
    }

    /// Returns the peer id of the network handle.
    pub fn peer_id(&self) -> &PeerId {
        self.inner.inner_network_handle.peer_id()
    }

    /// Sends a message to the network manager.
    pub fn send_message(&self, msg: NetworkHandleMessage) {
        let _ = self.inner.to_manager_tx.send(msg);
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

    /// Returns the secret key of the network handle.
    pub fn secret_key(&self) -> &SecretKey {
        self.inner.inner_network_handle.secret_key()
    }

    pub fn local_node_record(&self) -> reth_network_peers::NodeRecord {
        self.inner.inner_network_handle.local_node_record()
    }
}

/// A message type used for communication between the [`NetworkHandle`] and the
/// [`super::NetworkManager`].
pub enum NetworkHandleMessage {
    AnnounceBlock { block: ScrollBlock, signature: Signature },
    Shutdown(oneshot::Sender<()>),
}
