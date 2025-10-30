//! Command handle for the L1 Watcher.

mod command;

pub use command::L1WatcherCommand;

use crate::L1Notification;
use std::sync::Arc;
use tokio::sync::{mpsc, mpsc::UnboundedSender, oneshot};

/// Handle to interact with the L1 Watcher.
#[derive(Debug)]
pub struct L1WatcherHandle {
    to_watcher_tx: UnboundedSender<L1WatcherCommand>,
}

impl L1WatcherHandle {
    /// Create a new handle with the given command sender.
    pub const fn new(to_watcher_tx: UnboundedSender<L1WatcherCommand>) -> Self {
        Self { to_watcher_tx }
    }

    /// Send a command to the watcher without waiting for a response.
    fn send_command(&self, command: L1WatcherCommand) {
        if let Err(err) = self.to_watcher_tx.send(command) {
            tracing::error!(target: "scroll::watcher", ?err, "Failed to send command to L1 watcher");
        }
    }

    /// Reset the L1 Watcher to a specific block number with a fresh notification channel.
    ///
    /// Returns an error if the command could not be delivered or the watcher
    /// dropped the response channel.
    pub async fn reset_to_block(
        &self,
        block: u64,
        new_sender: mpsc::Sender<Arc<L1Notification>>,
    ) -> Result<(), oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(L1WatcherCommand::ResetToBlock {
            block,
            new_sender,
            response_sender: tx,
        });
        rx.await
    }
}
