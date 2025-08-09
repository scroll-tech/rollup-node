use super::{RollupManagerCommand, RollupManagerEvent};
use reth_network_api::FullNetwork;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_tokio_util::EventStream;
use scroll_network::ScrollNetworkHandle;
use tokio::sync::{mpsc, oneshot};

/// The handle used to send commands to the rollup manager.
#[derive(Debug, Clone)]
pub struct RollupManagerHandle<N: FullNetwork<Primitives = ScrollNetworkPrimitives>> {
    /// The channel used to send commands to the rollup manager.
    to_manager_tx: mpsc::Sender<RollupManagerCommand<N>>,
}

impl<N: FullNetwork<Primitives = ScrollNetworkPrimitives>> RollupManagerHandle<N> {
    /// Create a new rollup manager handle.
    pub const fn new(to_manager_tx: mpsc::Sender<RollupManagerCommand<N>>) -> Self {
        Self { to_manager_tx }
    }

    /// Sends a command to the rollup manager.
    pub async fn send_command(&self, command: RollupManagerCommand<N>) {
        let _ = self.to_manager_tx.send(command).await;
    }

    /// Sends a command to the rollup manager to build a block.
    pub async fn build_block(&self) {
        self.send_command(RollupManagerCommand::BuildBlock).await;
    }

    /// Sends a command to the rollup manager to get the network handle.
    pub async fn get_network_handle(
        &self,
    ) -> Result<ScrollNetworkHandle<N>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(RollupManagerCommand::NetworkHandle(tx)).await;
        rx.await
    }

    /// Sends a command to the rollup manager to fetch an event listener for the rollup node
    /// manager.
    pub async fn get_event_listener(
        &self,
    ) -> Result<EventStream<RollupManagerEvent>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(RollupManagerCommand::EventListener(tx)).await;
        rx.await
    }
}
