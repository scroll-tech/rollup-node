use crate::ChainOrchestratorStatus;

use super::ChainOrchestratorEvent;
// use crate::manager::metrics::HandleMetrics;
use reth_network_api::FullNetwork;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_tokio_util::EventStream;
use rollup_node_primitives::{BlockInfo, L1MessageEnvelope};
use scroll_db::L1MessageKey;
use scroll_network::ScrollNetworkHandle;
use tokio::sync::{mpsc, oneshot};
use tracing::error;

mod command;
pub use command::{ChainOrchestratorCommand, DatabaseQuery};

mod metrics;
use metrics::ChainOrchestratorHandleMetrics;

/// The handle used to send commands to the rollup manager.
#[derive(Debug, Clone)]
pub struct ChainOrchestratorHandle<N: FullNetwork<Primitives = ScrollNetworkPrimitives>> {
    /// The channel used to send commands to the rollup manager.
    to_manager_tx: mpsc::UnboundedSender<ChainOrchestratorCommand<N>>,
    /// The metrics for the handle.
    handle_metrics: ChainOrchestratorHandleMetrics,
    /// Mock for the L1 Watcher used in tests.
    #[cfg(feature = "test-utils")]
    pub l1_watcher_mock: Option<rollup_node_watcher::test_utils::L1WatcherMock>,
}

impl<N: FullNetwork<Primitives = ScrollNetworkPrimitives>> ChainOrchestratorHandle<N> {
    /// Create a new rollup manager handle.
    pub fn new(to_manager_tx: mpsc::UnboundedSender<ChainOrchestratorCommand<N>>) -> Self {
        Self {
            to_manager_tx,
            handle_metrics: ChainOrchestratorHandleMetrics::default(),
            #[cfg(feature = "test-utils")]
            l1_watcher_mock: None,
        }
    }

    /// Sets the L1 watcher mock for the handle.
    #[cfg(feature = "test-utils")]
    pub fn with_l1_watcher_mock(
        mut self,
        l1_watcher_mock: Option<rollup_node_watcher::test_utils::L1WatcherMock>,
    ) -> Self {
        self.l1_watcher_mock = l1_watcher_mock;
        self
    }

    /// Sends a command to the rollup manager.
    pub fn send_command(&self, command: ChainOrchestratorCommand<N>) {
        if let Err(err) = self.to_manager_tx.send(command) {
            self.handle_metrics.handle_send_command_failed.increment(1);
            error!(target: "rollup::manager::handle", "Failed to send command to rollup manager: {}", err);
        }
    }

    /// Sends a command to the rollup manager to build a block.
    pub fn build_block(&self) {
        self.send_command(ChainOrchestratorCommand::BuildBlock);
    }

    /// Sends a command to the rollup manager to get the network handle.
    pub async fn get_network_handle(
        &self,
    ) -> Result<ScrollNetworkHandle<N>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::NetworkHandle(tx));
        rx.await
    }

    /// Sends a command to the rollup manager to fetch an event listener for the rollup node
    /// manager.
    pub async fn get_event_listener(
        &self,
    ) -> Result<EventStream<ChainOrchestratorEvent>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::EventListener(tx));
        rx.await
    }

    /// Sends a command to the rollup manager to update the head of the FCS in the engine driver.
    pub async fn update_fcs_head(&self, head: BlockInfo) -> Result<(), oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::UpdateFcsHead((head, tx)));
        rx.await
    }

    /// Sends a command to the rollup manager to enable automatic sequencing.
    pub async fn enable_automatic_sequencing(&self) -> Result<bool, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::EnableAutomaticSequencing(tx));
        rx.await
    }

    /// Sends a command to the rollup manager to disable automatic sequencing.
    pub async fn disable_automatic_sequencing(&self) -> Result<bool, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::DisableAutomaticSequencing(tx));
        rx.await
    }

    /// Sends a command to the rollup manager to get the current status.
    pub async fn status(&self) -> Result<ChainOrchestratorStatus, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::Status(tx));
        rx.await
    }

    /// Get an L1 message by its index.
    pub async fn get_l1_message_by_key(
        &self,
        key: L1MessageKey,
    ) -> Result<Option<L1MessageEnvelope>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::DatabaseQuery(
            DatabaseQuery::GetL1MessageByKey(key, tx),
        ));
        rx.await
    }

    /// Revert the rollup node state to the specified L1 block number.
    pub async fn revert_to_l1_block(
        &self,
        block_number: u64,
    ) -> Result<bool, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::RevertToL1Block((block_number, tx)));
        rx.await
    }

    /// Sends a command to the rollup manager to enable or disable gossiping of blocks to peers.
    #[cfg(feature = "test-utils")]
    pub async fn set_gossip(&self, enabled: bool) -> Result<(), oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::SetGossip((enabled, tx)));
        rx.await
    }

    /// Sends a command to the rollup manager to get a database handle for direct database access.
    #[cfg(feature = "test-utils")]
    pub async fn get_database_handle(
        &self,
    ) -> Result<std::sync::Arc<scroll_db::Database>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(ChainOrchestratorCommand::DatabaseHandle(tx));
        rx.await
    }
}
