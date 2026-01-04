use crate::L1Notification;
use std::sync::Arc;
use tokio::sync::{mpsc, mpsc::UnboundedSender};

mod command;
pub use command::L1WatcherCommand;

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
    pub const fn l1_notification_receiver(&mut self) -> &mut mpsc::Receiver<Arc<L1Notification>> {
        &mut self.l1_notification_rx
    }

    /// Send a command to the watcher.
    fn send_command(&self, command: L1WatcherCommand) {
        if let Err(err) = self.to_watcher_tx.send(command) {
            tracing::error!(target: "scroll::watcher", ?err, "Failed to send command to L1 watcher");
        }
    }

    /// Reset the L1 Watcher to a specific block number with a fresh notification channel.
    pub fn revert_to_l1_block(&mut self, block: u64) {
        // Create a fresh notification channel with the same capacity as the original channel
        let capacity = self.l1_notification_rx.max_capacity();
        let (tx, rx) = mpsc::channel(capacity);

        // Send the reset command to the watcher
        self.send_command(L1WatcherCommand::ResetToBlock { block, tx });

        // Replace the old receiver with the new one
        self.l1_notification_rx = rx;
    }
}
