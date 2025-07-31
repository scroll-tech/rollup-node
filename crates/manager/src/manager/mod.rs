//! The [`RollupNodeManager`] is the main component of the rollup node that manages the
//! [`ScrollNetworkManager`], [`EngineDriver`], [`ChainOrchestrator`] and [`Consensus`] components.
//! It is responsible for handling events from these components and coordinating their actions.

use super::Consensus;
use alloy_provider::Provider;
use futures::StreamExt;
use reth_chainspec::EthChainSpec;
use reth_network::BlockDownloaderProvider;
use reth_network_api::FullNetwork;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_tasks::shutdown::GracefulShutdown;
use reth_tokio_util::{EventSender, EventStream};
use rollup_node_chain_orchestrator::{
    ChainOrchestrator, ChainOrchestratorError, ChainOrchestratorEvent,
};
use rollup_node_primitives::BlockInfo;
use rollup_node_sequencer::Sequencer;
use rollup_node_signer::{SignerEvent, SignerHandle};
use rollup_node_watcher::L1Notification;
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_network::Scroll;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_engine::{EngineDriver, EngineDriverEvent, ForkchoiceState};
use scroll_network::{
    BlockImportOutcome, NetworkManagerEvent, NewBlockWithPeer, ScrollNetworkManager,
};
use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    sync::mpsc::{self, Receiver},
    time::Interval,
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, trace, warn};

use rollup_node_providers::{L1MessageProvider, L1Provider};
use scroll_db::{Database, DatabaseError};
use scroll_derivation_pipeline::DerivationPipeline;

mod command;
pub use command::RollupManagerCommand;

mod event;
pub use event::RollupManagerEvent;

mod handle;
pub use handle::RollupManagerHandle;

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
pub struct RollupNodeManager<
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
    EC,
    P,
    L1P,
    L1MP,
    CS,
> {
    /// The handle receiver used to receive commands.
    handle_rx: Receiver<RollupManagerCommand>,
    /// The chain spec used by the rollup node.
    chain_spec: Arc<CS>,
    /// The network manager that manages the scroll p2p network.
    network: ScrollNetworkManager<N, CS>,
    /// The engine driver used to communicate with the engine.
    engine: EngineDriver<EC, CS, P>,
    /// The derivation pipeline, used to derive payload attributes from batches.
    derivation_pipeline: DerivationPipeline<L1P>,
    /// A receiver for [`L1Notification`]s from the [`rollup_node_watcher::L1Watcher`].
    l1_notification_rx: Option<ReceiverStream<Arc<L1Notification>>>,
    /// The chain orchestrator.
    chain: ChainOrchestrator<CS, <N as BlockDownloaderProvider>::Client, P>,
    /// The consensus algorithm used by the rollup node.
    consensus: Box<dyn Consensus>,
    /// An event sender for sending events to subscribers of the rollup node manager.
    event_sender: Option<EventSender<RollupManagerEvent>>,
    /// The sequencer which is responsible for sequencing transactions and producing new blocks.
    sequencer: Option<Sequencer<L1MP>>,
    /// The signer handle used to sign artifacts.
    signer: Option<SignerHandle>,
    /// The trigger for the block building process.
    block_building_trigger: Option<Interval>,
}

/// The current status of the rollup manager.
#[derive(Debug)]
pub struct RollupManagerStatus {
    /// Whether the rollup manager is syncing.
    pub syncing: bool,
    /// The current FCS for the manager.
    pub forkchoice_state: ForkchoiceState,
}

impl<
        N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
        EC: Debug,
        P: Debug,
        L1P: Debug,
        L1MP: Debug,
        CS: Debug,
    > Debug for RollupNodeManager<N, EC, P, L1P, L1MP, CS>
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RollupNodeManager")
            .field("chain_spec", &self.chain_spec)
            .field("network", &self.network)
            .field("engine", &self.engine)
            .field("derivation_pipeline", &self.derivation_pipeline)
            .field("l1_notification_rx", &self.l1_notification_rx)
            .field("indexer", &self.chain)
            .field("consensus", &self.consensus)
            .field("eth_wire_block_rx", &"eth_wire_block_rx")
            .field("event_sender", &self.event_sender)
            .field("sequencer", &self.sequencer)
            .field("block_building_trigger", &self.block_building_trigger)
            .finish()
    }
}

impl<N, EC, P, L1P, L1MP, CS> RollupNodeManager<N, EC, P, L1P, L1MP, CS>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
    EC: ScrollEngineApi + Unpin + Sync + Send + 'static,
    P: Provider<Scroll> + Clone + Unpin + Send + Sync + 'static,
    L1P: L1Provider + Clone + Send + Sync + Unpin + 'static,
    L1MP: L1MessageProvider + Unpin + Send + Sync + 'static,
    CS: ScrollHardforks + EthChainSpec + Send + Sync + 'static,
{
    /// Create a new [`RollupNodeManager`] instance.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::new_ret_no_self)]
    pub async fn new(
        network: ScrollNetworkManager<N, CS>,
        engine: EngineDriver<EC, CS, P>,
        l1_provider: L1P,
        database: Arc<Database>,
        l1_notification_rx: Option<Receiver<Arc<L1Notification>>>,
        consensus: Box<dyn Consensus>,
        chain_spec: Arc<CS>,
        sequencer: Option<Sequencer<L1MP>>,
        signer: Option<SignerHandle>,
        block_time: Option<u64>,
        chain_orchestrator: ChainOrchestrator<CS, <N as BlockDownloaderProvider>::Client, P>,
    ) -> (Self, RollupManagerHandle) {
        let (handle_tx, handle_rx) = mpsc::channel(EVENT_CHANNEL_SIZE);
        let derivation_pipeline = DerivationPipeline::new(l1_provider, database);
        let rnm = Self {
            handle_rx,
            chain_spec,
            network,
            engine,
            derivation_pipeline,
            l1_notification_rx: l1_notification_rx.map(Into::into),
            chain: chain_orchestrator,
            consensus,
            event_sender: None,
            sequencer,
            signer,
            block_building_trigger: block_time.map(delayed_interval),
        };
        (rnm, RollupManagerHandle::new(handle_tx))
    }

    /// Returns a new event listener for the rollup node manager.
    pub fn event_listener(&mut self) -> EventStream<RollupManagerEvent> {
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
            event_sender.notify(RollupManagerEvent::NewBlockReceived(block_with_peer.clone()));
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
            self.chain.handle_block_from_peer(block_with_peer);
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
    fn handle_chain_orchestrator_event(&mut self, event: ChainOrchestratorEvent) {
        trace!(target: "scroll::node::manager", ?event, "Received chain orchestrator event");

        if let Some(event_sender) = self.event_sender.as_ref() {
            event_sender.notify(RollupManagerEvent::ChainOrchestratorEvent(event.clone()));
        }

        match event {
            ChainOrchestratorEvent::BatchCommitIndexed {
                batch_info,
                safe_head,
                l1_block_number,
            } => {
                // if we detected a batch revert event, we reset the pipeline and the engine driver.
                if let Some(new_safe_head) = safe_head {
                    self.derivation_pipeline.flush();
                    self.engine.clear_l1_payload_attributes();
                    self.engine.set_head_block_info(new_safe_head);
                    self.engine.set_safe_block_info(new_safe_head);
                }
                // push the batch info into the derivation pipeline.
                self.derivation_pipeline.handle_batch_commit(batch_info, l1_block_number);
            }
            ChainOrchestratorEvent::BatchFinalized(_, Some(finalized_block)) => {
                // update the fcs on new finalized block.
                self.engine.set_finalized_block_info(finalized_block);
            }
            ChainOrchestratorEvent::L1BlockFinalized(l1_block_number, finalized_block) => {
                if let Some(sequencer) = self.sequencer.as_mut() {
                    sequencer.set_l1_finalized_block_number(l1_block_number);
                }
                // update the fcs on new finalized block.
                if let Some(finalized_block) = finalized_block {
                    self.engine.set_finalized_block_info(finalized_block);
                }
            }
            ChainOrchestratorEvent::ChainUnwound {
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

                // Handle the reorg in the derivation pipeline.
                self.derivation_pipeline.handle_reorg(l1_block_number);

                if let Some(event_sender) = self.event_sender.as_ref() {
                    event_sender.notify(RollupManagerEvent::Reorg(l1_block_number));
                }
            }
            ChainOrchestratorEvent::ChainExtended(chain_import) => {
                trace!(target: "scroll::node::manager", head = ?chain_import.chain.last().unwrap().header.clone(), peer_id = ?chain_import.peer_id.clone(),  "Received chain extension from peer");
                // Issue the new chain to the engine driver for processing.
                self.engine.handle_chain_import(chain_import)
            }
            ChainOrchestratorEvent::ChainReorged(chain_import) => {
                trace!(target: "scroll::node::manager", head = ?chain_import.chain.last().unwrap().header, ?chain_import.peer_id, "Received chain reorg from peer");

                // Issue the new chain to the engine driver for processing.
                self.engine.handle_chain_import(chain_import)
            }
            ChainOrchestratorEvent::OptimisticSync(block) => {
                let block_info: BlockInfo = (&block).into();
                trace!(target: "scroll::node::manager", ?block_info, "Received optimistic sync from peer");

                // Issue the new block info to the engine driver for processing.
                self.engine.handle_optimistic_sync(block_info)
            }
            _ => {}
        }
    }

    /// Handles an engine driver event.
    fn handle_engine_driver_event(&mut self, event: EngineDriverEvent) {
        trace!(target: "scroll::node::manager", ?event, "Received engine driver event");
        match event {
            EngineDriverEvent::BlockImportOutcome(outcome) => {
                if let Some(block) = outcome.block() {
                    if let Some(event_sender) = self.event_sender.as_ref() {
                        event_sender.notify(RollupManagerEvent::BlockImported(block.clone()));
                    }
                    self.chain.consolidate_l2_blocks(vec![(&block).into()], None);
                }
                self.network.handle().block_import_outcome(outcome);
            }
            EngineDriverEvent::NewPayload(payload) => {
                if let Some(signer) = self.signer.as_mut() {
                    let _ = signer.sign_block(payload.clone()).inspect_err(|err| error!(target: "scroll::node::manager", ?err, "Failed to send new payload to signer"));
                }

                if let Some(event_sender) = self.event_sender.as_ref() {
                    event_sender.notify(RollupManagerEvent::BlockSequenced(payload.clone()));
                }

                self.chain.consolidate_l2_blocks(vec![(&payload).into()], None);
            }
            EngineDriverEvent::L1BlockConsolidated(consolidation_outcome) => {
                self.chain.consolidate_l2_blocks(
                    vec![consolidation_outcome.block_info().clone()],
                    Some(*consolidation_outcome.batch_info()),
                );

                if let Some(event_sender) = self.event_sender.as_ref() {
                    event_sender.notify(RollupManagerEvent::L1DerivedBlockConsolidated(
                        consolidation_outcome,
                    ));
                }
            }
            EngineDriverEvent::ChainImportOutcome(outcome) => {
                if let Some(block) = outcome.outcome.block() {
                    if let Some(event_sender) = self.event_sender.as_ref() {
                        event_sender.notify(RollupManagerEvent::BlockImported(block));
                    }
                    self.chain.consolidate_l2_blocks(
                        outcome.chain.iter().map(|b| b.into()).collect(),
                        None,
                    );
                }
                self.network.handle().block_import_outcome(outcome.outcome);
            }
        }
    }

    /// Handles an [`L1Notification`] from the L1 watcher.
    fn handle_l1_notification(&mut self, notification: L1Notification) {
        if let Some(event_sender) = self.event_sender.as_ref() {
            event_sender.notify(RollupManagerEvent::L1NotificationEvent(notification.clone()));
        }

        match notification {
            L1Notification::Consensus(ref update) => self.consensus.update_config(update),
            L1Notification::NewBlock(new_block) => {
                if let Some(sequencer) = self.sequencer.as_mut() {
                    sequencer.handle_new_l1_block(new_block)
                }
            }
            L1Notification::Synced => {
                self.chain.handle_l1_notification(L1Notification::Synced);
            }
            _ => self.chain.handle_l1_notification(notification),
        }
    }

    /// Returns the current status of the [`RollupNodeManager`].
    fn status(&self) -> RollupManagerStatus {
        RollupManagerStatus {
            syncing: self.engine.is_syncing(),
            forkchoice_state: self.engine.forkchoice_state().clone(),
        }
    }

    /// Drives the [`RollupNodeManager`] future until a [`GracefulShutdown`] signal is received.
    pub async fn run_until_graceful_shutdown(mut self, shutdown: GracefulShutdown) {
        let mut graceful_guard = None;

        tokio::select! {
            _ = &mut self => {},
            guard = shutdown => {
                graceful_guard = Some(guard);
            },
        }

        drop(graceful_guard);
    }
}

impl<N, EC, P, L1P, L1MP, CS> Future for RollupNodeManager<N, EC, P, L1P, L1MP, CS>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
    EC: ScrollEngineApi + Unpin + Sync + Send + 'static,
    P: Provider<Scroll> + Clone + Unpin + Send + Sync + 'static,
    L1P: L1Provider + Clone + Unpin + Send + Sync + 'static,
    L1MP: L1MessageProvider + Unpin + Send + Sync + 'static,
    CS: ScrollHardforks + EthChainSpec + Unpin + Send + Sync + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        // Helper macro, proceeds with the $task if $proceed is true.
        macro_rules! proceed_if {
            ($proceed: expr, $task: expr) => {
                if $proceed {
                    $task
                }
            };
        }
        let en_synced = !this.engine.is_syncing();

        // Poll the handle receiver for commands.
        while let Poll::Ready(Some(command)) = this.handle_rx.poll_recv(cx) {
            match command {
                RollupManagerCommand::BuildBlock => {
                    proceed_if!(
                        en_synced,
                        if let Some(sequencer) = this.sequencer.as_mut() {
                            sequencer.build_payload_attributes();
                        }
                    );
                }
                RollupManagerCommand::EventListener(tx) => {
                    let events = this.event_listener();
                    tx.send(events).expect("Failed to send event listener to handle");
                }
                RollupManagerCommand::Status(tx) => {
                    tx.send(this.status()).expect("Failed to send status to handle");
                }
                RollupManagerCommand::UpdateFcsHead(head) => {
                    trace!(target: "scroll::node::manager", ?head, "Updating FCS head block info");
                    this.engine.set_head_block_info(head);
                }
            }
        }

        // Drain all EngineDriver events.
        while let Poll::Ready(Some(event)) = this.engine.poll_next_unpin(cx) {
            this.handle_engine_driver_event(event);
        }

        proceed_if!(
            en_synced,
            // Handle new block production.
            if let Some(Poll::Ready(Some(attributes))) =
                this.sequencer.as_mut().map(|x| x.poll_next_unpin(cx))
            {
                this.engine.handle_build_new_payload(attributes);
            }
        );

        proceed_if!(
            en_synced,
            // Drain all L1 notifications.
            while let Some(Poll::Ready(Some(event))) =
                this.l1_notification_rx.as_mut().map(|rx| rx.poll_next_unpin(cx))
            {
                this.handle_l1_notification((*event).clone());
            }
        );

        // Drain all Indexer events.
        while let Poll::Ready(Some(result)) = this.chain.poll_next_unpin(cx) {
            match result {
                Ok(event) => this.handle_chain_orchestrator_event(event),
                Err(err) => {
                    match &err {
                        ChainOrchestratorError::L1MessageMismatch { expected, actual } => {
                            if let Some(event_sender) = this.event_sender.as_ref() {
                                event_sender.notify(
                                    RollupManagerEvent::L1MessageConsolidationError {
                                        expected: *expected,
                                        actual: *actual,
                                    },
                                );
                            }
                        }
                        ChainOrchestratorError::DatabaseError(
                            DatabaseError::L1MessageNotFound(start),
                        ) => {
                            if let Some(event_sender) = this.event_sender.as_ref() {
                                event_sender.notify(
                                    RollupManagerEvent::L1MessageMissingInDatabase {
                                        start: start.clone(),
                                    },
                                );
                            }
                        }
                        _ => {}
                    }

                    error!(
                        target: "scroll::node::manager",
                        ?err,
                        "Error occurred in the chain orchestrator"
                    );
                }
            }
        }

        // Drain all signer events.
        while let Some(Poll::Ready(Some(event))) =
            this.signer.as_mut().map(|s| s.poll_next_unpin(cx))
        {
            match event {
                SignerEvent::SignedBlock { block, signature } => {
                    trace!(target: "scroll::node::manager", ?block, ?signature, "Received signed block from signer, announcing to the network");
                    // Send SignerEvent for test monitoring
                    if let Some(event_sender) = this.event_sender.as_ref() {
                        event_sender.notify(RollupManagerEvent::SignerEvent(
                            SignerEvent::SignedBlock { block: block.clone(), signature },
                        ));
                    }

                    this.network.handle().announce_block(block, signature);
                }
            }
        }

        proceed_if!(
            en_synced,
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
        );

        // Poll Derivation Pipeline and push attribute in queue if any.
        while let Poll::Ready(Some(attributes)) = this.derivation_pipeline.poll_next_unpin(cx) {
            this.engine.handle_l1_consolidation(attributes)
        }

        // Handle network manager events.
        while let Poll::Ready(Some(event)) = this.network.poll_next_unpin(cx) {
            this.handle_network_manager_event(event);
        }

        Poll::Pending
    }
}

/// Creates a delayed interval that will not skip ticks if the interval is missed but will delay
/// the next tick until the interval has passed.
fn delayed_interval(interval: u64) -> Interval {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(interval));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}
