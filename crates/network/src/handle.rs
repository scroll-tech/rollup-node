use reth_network::NetworkHandle as RethNetworkHandle;
use reth_network_peers::PeerId;
use reth_primitives::Block;
use secp256k1::{ecdsa::Signature, SecretKey};
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

/// A _sharable_ frontend used to communicate with the [`NetworkManager`].
#[derive(Debug, Clone)]
pub struct NetworkHandle {
    /// A reference to the inner network handle.
    pub(crate) inner: Arc<NetworkInner>,
}

impl NetworkHandle {
    /// Creates a new [`NetworkHandle`] instance from the given [`UnboundedSender`] and [`RethNetworkHandle`].
    pub fn new(
        to_manager_tx: UnboundedSender<NetworkHandleMessage>,
        inner_network_handle: RethNetworkHandle,
    ) -> Self {
        let inner = NetworkInner {
            to_manager_tx,
            inner_network_handle,
        };
        Self {
            inner: Arc::new(inner),
        }
    }
}

/// The inner state of the [`NetworkHandle`].
#[derive(Debug)]
pub struct NetworkInner {
    /// The sender half of the channel set up between this type and the [`NetworkManager`].
    pub(crate) to_manager_tx: UnboundedSender<NetworkHandleMessage>,
    /// A reference to the inner network handle which is used to communicate with the inner network.
    pub inner_network_handle: RethNetworkHandle,
}

impl NetworkHandle {
    /// Returns a reference to the inner network handle.
    pub fn inner(&self) -> &RethNetworkHandle {
        &self.inner.inner_network_handle
    }

    /// Returns the peer id of the network handle.
    pub fn peer_id(&self) -> &PeerId {
        &self.inner.inner_network_handle.peer_id()
    }

    /// Sends a message to the network manager.
    pub fn send_message(&self, msg: NetworkHandleMessage) {
        let _ = self.inner.to_manager_tx.send(msg);
    }

    /// Announces a block to the network.
    pub fn announce_block(&self, block: Block, signature: Signature) {
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
        &self.inner.inner_network_handle.secret_key()
    }
}

/// A message type used for communication between the [`NetworkHandle`] and the [`super::NetworkManager`].
pub enum NetworkHandleMessage {
    AnnounceBlock { block: Block, signature: Signature },
    Shutdown(oneshot::Sender<()>),
}
