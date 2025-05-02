//! This library contains the main manager for the rollup node.

use alloy_primitives::Signature;
use futures::StreamExt;
use reth_chainspec::EthChainSpec;
use reth_tokio_util::{EventSender, EventStream};
use rollup_node_indexer::{Indexer, IndexerEvent};
use rollup_node_sequencer::Sequencer;
use rollup_node_watcher::L1Notification;
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_engine::{EngineDriver, EngineDriverEvent};
use scroll_network::{BlockImportOutcome, NetworkManager, NetworkManagerEvent, NewBlockWithPeer};
use std::{
    fmt,
    fmt::{Debug, Formatter},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    sync::mpsc::{Receiver, UnboundedReceiver},
    time::Interval,
};
use tokio_stream::wrappers::{ReceiverStream, UnboundedReceiverStream};
use tracing::{error, trace, warn};

pub use event::RollupEvent;
mod event;

pub use consensus::{Consensus, NoopConsensus, PoAConsensus};
mod consensus;

use rollup_node_providers::{ExecutionPayloadProvider, L1MessageProvider, L1Provider};
use scroll_db::Database;
use scroll_derivation_pipeline::DerivationPipeline;

/// The size of the event channel.
const EVENT_CHANNEL_SIZE: usize = 100;

/// The main manager for the rollup node.
///
/// This is an endless [`Future`] that drives the state of the entire network forward and includes
/// the following components:
/// - `network`: Responsible for peer discover, managing connections between peers and operation of
///   the eth-wire protocol.
/// - `engine`: Responsible for importing blocks that have been gossiped over the scroll-wire
///   protocol.
/// - `consensus`: The consensus algorithm used by the rollup node.
/// - `new_block_rx`: Receives new blocks from the network.
/// - `forkchoice_state`: The forkchoice state of the rollup node.
/// - `pending_block_imports`: A collection of pending block imports.
/// - `event_sender`: An event sender for sending events to subscribers of the rollup node manager.
pub struct RollupNodeManager<EC, P, L1P, L1MP, CS> {
    /// The network manager that manages the scroll p2p network.
    network: NetworkManager,
    /// The engine driver used to communicate with the engine.
    engine: EngineDriver<EC, P>,
    /// The derivation pipeline, used to derive payload attributes from batches.
    derivation_pipeline: Option<DerivationPipeline<L1P>>,
    /// A receiver for [`L1Notification`]s from the [`rollup_node_watcher::L1Watcher`].
    l1_notification_rx: Option<ReceiverStream<Arc<L1Notification>>>,
    /// An indexer used to index data for the rollup node.
    indexer: Indexer<CS>,
    /// The consensus algorithm used by the rollup node.
    consensus: Box<dyn Consensus>,
    /// The receiver for new blocks received from the network (used to bridge from eth-wire).
    new_block_rx: Option<UnboundedReceiverStream<NewBlockWithPeer>>,
    /// An event sender for sending events to subscribers of the rollup node manager.
    event_sender: Option<EventSender<RollupEvent>>,
    /// The sequencer which is responsible for sequencing transactions and producing new blocks.
    sequencer: Option<Sequencer<L1MP>>,
    /// The trigger for the block building process.
    block_building_trigger: Option<Interval>,
}

impl<EC: Debug, P: Debug, L1P: Debug, L1MP: Debug, CS: Debug> Debug
    for RollupNodeManager<EC, P, L1P, L1MP, CS>
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RollupNodeManager")
            .field("network", &self.network)
            .field("engine", &self.engine)
            .field("derivation_pipeline", &self.derivation_pipeline)
            .field("l1_notification_rx", &self.l1_notification_rx)
            .field("indexer", &self.indexer)
            .field("consensus", &self.consensus)
            .field("new_block_rx", &self.new_block_rx)
            .field("event_sender", &self.event_sender)
            .field("sequencer", &self.sequencer)
            .field("block_building_trigger", &self.block_building_trigger)
            .finish()
    }
}

impl<EC, P, L1P, L1MP, CS> RollupNodeManager<EC, P, L1P, L1MP, CS>
where
    EC: ScrollEngineApi + Unpin + Sync + Send + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
    L1P: L1Provider + Clone + Send + Sync + 'static,
    L1MP: L1MessageProvider + Unpin + Send + Sync + 'static,
    CS: ScrollHardforks + EthChainSpec + Send + Sync + 'static,
{
    /// Create a new [`RollupNodeManager`] instance.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        network: NetworkManager,
        engine: EngineDriver<EC, P>,
        l1_provider: Option<L1P>,
        database: Arc<Database>,
        l1_notification_rx: Option<Receiver<Arc<L1Notification>>>,
        consensus: Box<dyn Consensus>,
        chain_spec: Arc<CS>,
        new_block_rx: Option<UnboundedReceiver<NewBlockWithPeer>>,
        sequencer: Option<Sequencer<L1MP>>,
        block_time: Option<u64>,
    ) -> Self {
        let indexer = Indexer::new(database.clone(), chain_spec);
        let derivation_pipeline =
            l1_provider.map(|provider| DerivationPipeline::new(provider, database));
        Self {
            network,
            engine,
            derivation_pipeline,
            l1_notification_rx: l1_notification_rx.map(Into::into),
            indexer,
            consensus,
            new_block_rx: new_block_rx.map(Into::into),
            event_sender: None,
            sequencer,
            block_building_trigger: block_time
                .map(|time| tokio::time::interval(tokio::time::Duration::from_millis(time))),
        }
    }

    /// Returns a new event listener for the rollup node manager.
    pub fn event_listener(&mut self) -> EventStream<RollupEvent> {
        if let Some(event_sender) = &self.event_sender {
            return event_sender.new_listener()
        };

        let event_sender = EventSender::new(EVENT_CHANNEL_SIZE);
        let event_listener = event_sender.new_listener();
        self.event_sender = Some(event_sender);

        event_listener
    }

    /// Handles a new block received from the network.
    ///
    /// We will first validate the consensus of the block, then we will send the block to the engine
    /// to validate the correctness of the block.
    pub fn handle_new_block(&mut self, block_with_peer: NewBlockWithPeer) {
        trace!(target: "scroll::node::manager", "Received new block from peer {:?} - hash {:?}", block_with_peer.peer_id, block_with_peer.block.hash_slow());
        if let Some(event_sender) = self.event_sender.as_ref() {
            event_sender.notify(RollupEvent::NewBlockReceived(block_with_peer.clone()));
        }

        // Validate the consensus of the block.
        // TODO: Should we spawn a task to validate the consensus of the block?
        //       Is the consensus validation blocking?
        if let Err(err) =
            self.consensus.validate_new_block(&block_with_peer.block, &block_with_peer.signature)
        {
            error!(target: "scroll::node::manager", ?err, "consensus checks failed on block {:?} from peer {:?}", block_with_peer.block.hash_slow(), block_with_peer.peer_id);
            self.network.handle().block_import_outcome(BlockImportOutcome {
                peer: block_with_peer.peer_id,
                result: Err(err.into()),
            });
        } else {
            self.engine.handle_block_import(block_with_peer);
        }
    }

    /// Handles a network manager event.
    ///
    /// Currently the network manager only emits a `NewBlock` event.
    fn handle_network_manager_event(&mut self, event: NetworkManagerEvent) {
        match event {
            NetworkManagerEvent::NewBlock(block) => self.handle_new_block(block),
        }
    }

    /// Handles an indexer event.
    fn handle_indexer_event(&mut self, event: IndexerEvent) {
        trace!(target: "scroll::node::manager", ?event, "Received indexer event");
        match event {
            IndexerEvent::BatchCommitIndexed(batch_info) => {
                // push the batch info into the derivation pipeline.
                if let Some(pipeline) = &mut self.derivation_pipeline {
                    pipeline.handle_batch_commit(batch_info);
                }
            }
            IndexerEvent::BatchFinalizationIndexed(_, Some(finalized_block)) |
            IndexerEvent::FinalizedIndexed(_, Some(finalized_block)) => {
                // update the fcs on new finalized block.
                self.engine.set_finalized_block_info(finalized_block);
            }
            IndexerEvent::ReorgIndexed {
                l1_block_number,
                queue_index,
                l2_head_block_info,
                l2_safe_block_info,
            } => {
                // Update the [`EngineDriver`] fork choice state with the new L2 head info.
                if let Some(l2_head_block_info) = l2_head_block_info {
                    self.engine.set_head_block_info(l2_head_block_info);
                }

                // Update the [`EngineDriver`] fork choice state with the new L2 safe info.
                if let Some(safe_block_info) = l2_safe_block_info {
                    self.engine.set_safe_block_info(safe_block_info);
                }

                // Update the [`Sequencer`] with the new L1 head info and queue index.
                if let Some(sequencer) = self.sequencer.as_mut() {
                    sequencer.handle_reorg(queue_index, l1_block_number);
                }
            }
            _ => (),
        }
    }

    /// Handles an engine driver event.
    fn handle_engine_driver_event(&mut self, event: EngineDriverEvent) {
        trace!(target: "scroll::node::manager", ?event, "Received engine driver event");
        match event {
            EngineDriverEvent::BlockImportOutcome(outcome) => {
                if let Some(block) = outcome.block() {
                    self.indexer.handle_block(block.into(), None);
                }
                self.network.handle().block_import_outcome(outcome);
            }
            EngineDriverEvent::NewPayload(payload) => {
                // TODO: sign blocks before sending them to the network.
                let signature = Signature::from_raw(&[0; 65]).unwrap();
                self.indexer.handle_block(payload.clone().into(), None);
                self.network.handle().announce_block(payload, signature);
            }
            EngineDriverEvent::L1BlockConsolidated((block_info, batch_info)) => {
                self.indexer.handle_block(block_info.clone(), Some(batch_info));

                if let Some(event_sender) = self.event_sender.as_ref() {
                    event_sender.notify(RollupEvent::L1DerivedBlockConsolidated(block_info));
                }
            }
        }
    }

    /// Handles an [`L1Notification`] from the L1 watcher.
    fn handle_l1_notification(&mut self, notification: L1Notification) {
        self.indexer.handle_l1_notification(notification);
    }
}

impl<EC, P, L1P, L1MP, CS> Future for RollupNodeManager<EC, P, L1P, L1MP, CS>
where
    EC: ScrollEngineApi + Unpin + Sync + Send + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
    L1P: L1Provider + Clone + Unpin + Send + Sync + 'static,
    L1MP: L1MessageProvider + Unpin + Send + Sync + 'static,
    CS: ScrollHardforks + EthChainSpec + Unpin + Send + Sync + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Handle new block production.
        if let Some(Poll::Ready(Some(attributes))) =
            this.sequencer.as_mut().map(|x| x.poll_next_unpin(cx))
        {
            this.engine.handle_build_new_payload(attributes);
        }

        // Drain all EngineDriver events.
        while let Poll::Ready(Some(event)) = this.engine.poll_next_unpin(cx) {
            this.handle_engine_driver_event(event);
        }

        // Drain all L1 notifications.
        while let Some(Poll::Ready(Some(event))) =
            this.l1_notification_rx.as_mut().map(|rx| rx.poll_next_unpin(cx))
        {
            this.handle_l1_notification((*event).clone());
        }

        // Drain all Indexer events.
        while let Poll::Ready(Some(result)) = this.indexer.poll_next_unpin(cx) {
            match result {
                Ok(event) => this.handle_indexer_event(event),
                Err(err) => {
                    error!(target: "scroll::node::manager", ?err, "Error occurred at indexer level")
                }
            }
        }

        // Check if we need to trigger the build of a new payload.
        match (
            this.block_building_trigger.as_mut().map(|x| x.poll_tick(cx)),
            this.engine.is_payload_building_in_progress(),
        ) {
            (Some(Poll::Ready(_)), false) => {
                if let Some(sequencer) = this.sequencer.as_mut() {
                    sequencer.build_payload_attributes();
                }
            }
            (Some(Poll::Ready(_)), true) => {
                // If the sequencer is already building a payload, we don't need to trigger it
                // again.
                warn!(target: "scroll::node::manager", "Payload building is already in progress skipping slot");
            }
            _ => {}
        }

        // Poll Derivation Pipeline and push attribute in queue if any.
        while let Some(Poll::Ready(Some(attributes))) =
            this.derivation_pipeline.as_mut().map(|f| f.poll_next_unpin(cx))
        {
            this.engine.handle_l1_consolidation(attributes)
        }

        // Handle blocks received from the eth-wire protocol.
        while let Some(Poll::Ready(Some(block))) =
            this.new_block_rx.as_mut().map(|new_block_rx| new_block_rx.poll_next_unpin(cx))
        {
            this.handle_new_block(block);
        }

        // Handle network manager events.
        while let Poll::Ready(Some(event)) = this.network.poll_next_unpin(cx) {
            this.handle_network_manager_event(event);
        }

        Poll::Pending
    }
}
