use crate::L1Notification;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Commands that can be sent to the L1 Watcher.
#[derive(Debug, Clone)]
pub enum L1WatcherCommand {
    /// Reset the watcher to a specific L1 block number.
    ResetToBlock {
        /// The L1 block number to reset to.
        block: u64,
        /// New sender to replace the current notification channel.
        tx: mpsc::Sender<Arc<L1Notification>>,
    },
}
