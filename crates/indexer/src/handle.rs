use rollup_node_watcher::L1Notification;
use tokio::sync::mpsc::UnboundedSender;

/// A command to the indexer.
#[derive(Debug)]
pub enum IndexerCommand {
    /// A request to index an L1 notification.
    IndexL1Notification(L1Notification),
}

/// A handle which is used to send commands to the indexer.
#[derive(Debug)]
pub struct IndexerHandle {
    /// The sender half of the channel used to send commands to the indexer.
    cmd_tx: UnboundedSender<IndexerCommand>,
}

impl IndexerHandle {
    /// Creates a new indexer handle with the given command sender.
    pub const fn new(cmd_tx: UnboundedSender<IndexerCommand>) -> Self {
        Self { cmd_tx }
    }

    /// Sends a command to the indexer to index an L1 notification.
    pub fn index_l1_notification(&self, notification: L1Notification) {
        let _ = self.cmd_tx.send(IndexerCommand::IndexL1Notification(notification));
    }
}
