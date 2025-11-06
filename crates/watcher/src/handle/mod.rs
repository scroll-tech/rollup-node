//! Command handle for the L1 Watcher.

mod command;

pub use command::L1WatcherCommand;

use crate::L1Notification;
use std::sync::Arc;
use tokio::sync::{mpsc, mpsc::UnboundedSender};

/// Handle to interact with the L1 Watcher.
#[derive(Debug)]
pub struct L1WatcherHandle {
    to_watcher_tx: UnboundedSender<L1WatcherCommand>,
    l1_notification_rx: mpsc::Receiver<Arc<L1Notification>>,
}

impl L1WatcherHandle {
    /// Create a new handle with the given command sender.
    pub const fn new(
        to_watcher_tx: UnboundedSender<L1WatcherCommand>,
        l1_notification_rx: mpsc::Receiver<Arc<L1Notification>>,
    ) -> Self {
        Self { to_watcher_tx, l1_notification_rx }
    }

    /// Get a mutable reference to the L1 notification receiver.
    pub fn l1_notification_receiver(&mut self) -> &mut mpsc::Receiver<Arc<L1Notification>> {
        &mut self.l1_notification_rx
    }

    /// Send a command to the watcher without waiting for a response.
    fn send_command(&self, command: L1WatcherCommand) {
        if let Err(err) = self.to_watcher_tx.send(command) {
            tracing::error!(target: "scroll::watcher", ?err, "Failed to send command to L1 watcher");
        }
    }

    /// Triggers gap recovery by resetting the L1 watcher to a specific block with a fresh channel.
    pub async fn trigger_gap_recovery(&mut self, reset_block: u64) {
        // Create a fresh notification channel
        // Use the same capacity as the original channel
        let capacity = self.l1_notification_rx.max_capacity();
        let (new_tx, new_rx) = mpsc::channel(capacity);

        // Send reset command with the new sender and wait for confirmation
        self.reset_to_block(reset_block, new_tx).await;

        // Replace the receiver with the fresh channel
        // The old channel is automatically dropped, discarding all stale notifications
        self.l1_notification_rx = new_rx;
    }

    /// Reset the L1 Watcher to a specific block number with a fresh notification channel.
    async fn reset_to_block(&self, block: u64, new_sender: mpsc::Sender<Arc<L1Notification>>) {
        self.send_command(L1WatcherCommand::ResetToBlock { block, new_sender });
    }
}
