use super::{IndexerError, IndexerEvent};
use futures::{stream::StreamExt, Stream};
use rollup_node_primitives::{BatchInfo, L2BlockInfoWithL1Messages};
use rollup_node_watcher::L1Notification;
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// The commands that can be sent to the indexer.
#[derive(Debug)]
pub enum IndexerCommand {
    /// A command to handle an L1 notification.
    L1Notification(L1Notification),
    /// A command to handle a block.
    Block {
        /// The L2 block with L1 messages.
        block: L2BlockInfoWithL1Messages,
        /// Optional batch information.
        batch: Option<BatchInfo>,
    },
}

/// The handle for the indexer, allowing to send commands and receive events.
#[derive(Debug)]
pub struct IndexerHandle {
    command_tx: UnboundedSender<IndexerCommand>,
    event_rx: UnboundedReceiverStream<Result<IndexerEvent, IndexerError>>,
}

impl IndexerHandle {
    /// Creates a new [`IndexerHandle`] instance.
    pub const fn new(
        request_tx: UnboundedSender<IndexerCommand>,
        event_rx: UnboundedReceiverStream<Result<IndexerEvent, IndexerError>>,
    ) -> Self {
        Self { command_tx: request_tx, event_rx }
    }

    /// Sends a command to the indexer.
    pub fn handle_block(
        &self,
        block: L2BlockInfoWithL1Messages,
        batch: Option<BatchInfo>,
    ) -> Result<(), IndexerError> {
        self.command_tx
            .send(IndexerCommand::Block { block, batch })
            .map_err(|_| IndexerError::CommandChannelClosed)
    }

    /// Sends a L1 notification to the indexer.
    pub fn handle_l1_notification(&self, notification: L1Notification) -> Result<(), IndexerError> {
        self.command_tx
            .send(IndexerCommand::L1Notification(notification))
            .map_err(|_| IndexerError::CommandChannelClosed)
    }
}

impl Stream for IndexerHandle {
    type Item = Result<IndexerEvent, IndexerError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.get_mut().event_rx.poll_next_unpin(cx)
    }
}
