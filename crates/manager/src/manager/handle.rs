use super::{RollupManagerCommand, RollupManagerEvent};
use reth_tokio_util::EventStream;
use tokio::sync::{mpsc, oneshot};

/// The handle used to send commands to the rollup manager.
#[derive(Debug, Clone)]
pub struct RollupManagerHandle {
    /// The channel used to send commands to the rollup manager.
    to_manager_tx: mpsc::Sender<RollupManagerCommand>,
}

impl RollupManagerHandle {
    /// Create a new rollup manager handle.
    pub const fn new(to_manager_tx: mpsc::Sender<RollupManagerCommand>) -> Self {
        Self { to_manager_tx }
    }

    /// Sends a command to the rollup manager.
    pub async fn send_command(&self, command: RollupManagerCommand) {
        let _ = self.to_manager_tx.send(command).await;
    }

    /// Sends a command to the rollup manager to build a block.
    pub async fn build_block(&self) {
        self.send_command(RollupManagerCommand::BuildBlock).await;
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
