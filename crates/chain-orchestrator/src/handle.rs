use super::{ChainOrchestratorError, ChainOrchestratorEvent};
use futures::{stream::StreamExt, Stream};
use rollup_node_primitives::{BatchInfo, L2BlockInfoWithL1Messages};
use rollup_node_watcher::L1Notification;
use scroll_network::NewBlockWithPeer;
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// A handle to interact with the `ChainOrchestrator`.
#[derive(Debug)]
pub struct ChainOrchestratorHandle {
    /// The command sender to send commands to the `ChainOrchestrator`.
    command_sender: UnboundedSender<ChainOrchestratorCommand>,
    /// The event receiver to receive events from the `ChainOrchestrator`.
    event_receiver: UnboundedReceiverStream<Result<ChainOrchestratorEvent, ChainOrchestratorError>>,
}

impl ChainOrchestratorHandle {
    /// Creates a new [`ChainOrchestratorHandle`].
    pub const fn new(
        command_sender: UnboundedSender<ChainOrchestratorCommand>,
        event_receiver: UnboundedReceiverStream<
            Result<ChainOrchestratorEvent, ChainOrchestratorError>,
        >,
    ) -> Self {
        Self { command_sender, event_receiver }
    }

    /// Sends a new block received from a peer to the `ChainOrchestrator`.
    pub fn import_block_from_peer(
        &self,
        new_block: NewBlockWithPeer,
    ) -> Result<(), ChainOrchestratorError> {
        self.command_sender
            .send(ChainOrchestratorCommand::ImportBlockFromPeer(new_block))
            .map_err(|_| ChainOrchestratorError::CommandChannelClosed)
    }

    /// Sends a new block sequenced by this node to the `ChainOrchestrator`.
    pub fn import_sequenced_block(
        &self,
        new_block: NewBlockWithPeer,
    ) -> Result<(), ChainOrchestratorError> {
        self.command_sender
            .send(ChainOrchestratorCommand::ImportSequencedBlock(new_block))
            .map_err(|_| ChainOrchestratorError::CommandChannelClosed)
    }

    /// Sends a L1 notification to the `ChainOrchestrator`.
    pub fn handle_l1_notification(
        &self,
        event: L1Notification,
    ) -> Result<(), ChainOrchestratorError> {
        self.command_sender
            .send(ChainOrchestratorCommand::HandleL1Notification(event))
            .map_err(|_| ChainOrchestratorError::CommandChannelClosed)
    }

    /// Sends a vector of L2 blocks which have been consolidated from an L1 batch to be persisted.
    pub fn persist_consolidated_blocks(
        &self,
        l2_blocks: Vec<L2BlockInfoWithL1Messages>,
        batch_info: BatchInfo,
    ) -> Result<(), ChainOrchestratorError> {
        self.command_sender
            .send(ChainOrchestratorCommand::PersistConsolidatedL2Blocks(l2_blocks, batch_info))
            .map_err(|_| ChainOrchestratorError::CommandChannelClosed)
    }

    /// Sends a vector of L2 blocks which have been validated from the network to be persisted.
    pub fn persist_validated_l2_blocks(
        &self,
        l2_blocks: Vec<L2BlockInfoWithL1Messages>,
    ) -> Result<(), ChainOrchestratorError> {
        self.command_sender
            .send(ChainOrchestratorCommand::PersistValidatedL2Blocks(l2_blocks))
            .map_err(|_| ChainOrchestratorError::CommandChannelClosed)
    }
}

/// A command to be sent to the `ChainOrchestrator`.
#[derive(Debug)]
pub enum ChainOrchestratorCommand {
    /// Import a block from a peer.
    ImportBlockFromPeer(NewBlockWithPeer),
    /// Import a block sequenced by this node.
    ImportSequencedBlock(NewBlockWithPeer),
    /// Persist L2 blocks that have been consolidated from L1.
    PersistConsolidatedL2Blocks(Vec<L2BlockInfoWithL1Messages>, BatchInfo),
    /// Persist L2 blocks that have been validated from the network.
    PersistValidatedL2Blocks(Vec<L2BlockInfoWithL1Messages>),
    /// Handle an L1 notification.
    HandleL1Notification(L1Notification),
}

impl Stream for ChainOrchestratorHandle {
    type Item = Result<ChainOrchestratorEvent, ChainOrchestratorError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.event_receiver.poll_next_unpin(cx)
    }
}
