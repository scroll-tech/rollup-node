//! The [`RollupNodeManager`] is the main component of the rollup node that manages the
//! [`ScrollNetworkManager`], [`EngineDriver`], [`ChainOrchestrator`] and [`Consensus`] components.
//! It is responsible for handling events from these components and coordinating their actions.

use super::Consensus;
use crate::poll_nested_stream_with_budget;
use ::metrics::counter;
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
use tracing::{error, info, trace, warn};

use rollup_node_providers::{L1MessageProvider, L1Provider};
use scroll_db::{Database, DatabaseError, DatabaseTransactionProvider, DatabaseWriteOperations};
use scroll_derivation_pipeline::DerivationPipeline;

mod budget;
use budget::L1_NOTIFICATION_CHANNEL_BUDGET;

mod command;
pub use command::RollupManagerCommand;

mod event;
pub use event::RollupManagerEvent;

mod handle;
mod metrics;

use crate::manager::metrics::RollupNodeManagerMetrics;
pub use handle::RollupManagerHandle;

/// The size of the event channel.
const EVENT_CHANNEL_SIZE: usize = 100;

/// The maximum capacity of the pending futures queue in the chain orchestrator for acceptance of
/// new events from the L1 notification channel.
const CHAIN_ORCHESTRATOR_MAX_PENDING_FUTURES: usize = 20;

/// The maximum number of pending futures in the engine driver for acceptance of new events from the
/// L1 notification channel.
const ENGINE_MAX_PENDING_FUTURES: usize = 5000;

/// The maximum number of pending batch commits in the derivation pipeline for acceptance of new
/// events from the L1 notification channel.
const DERIVATION_PIPELINE_MAX_PENDING_BATCHES: usize = 500;

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
    handle_rx: Receiver<RollupManagerCommand<N>>,
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
    /// A connection to the database.
    database: Arc<Database>,
    /// The original block time configuration for restoring automatic sequencing.
    block_time_config: Option<u64>,
    /// Metrics for the rollup node manager.
    metrics: RollupNodeManagerMetrics,
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
            .field("chain_orchestrator", &self.chain)
            .field("consensus", &self.consensus)
            .field("eth_wire_block_rx", &"eth_wire_block_rx")
            .field("event_sender", &self.event_sender)
            .field("sequencer", &self.sequencer)
            .field("block_building_trigger", &self.block_building_trigger)
            .field("block_time_config", &self.block_time_config)
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
        auto_start: bool,
        chain_orchestrator: ChainOrchestrator<CS, <N as BlockDownloaderProvider>::Client, P>,
        l1_v2_message_queue_start_index: u64,
    ) -> (Self, RollupManagerHandle<N>) {
        let (handle_tx, handle_rx) = mpsc::channel(EVENT_CHANNEL_SIZE);
        let derivation_pipeline =
            DerivationPipeline::new(l1_provider, database.clone(), l1_v2_message_queue_start_index);
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
            block_building_trigger: if auto_start {
                block_time.map(delayed_interval)
            } else {
                None
            },
            database,
            block_time_config: block_time,
            metrics: RollupNodeManagerMetrics::default(),
        };
        (rnm, RollupManagerHandle::new(handle_tx))
    }

    /// Returns a new event listener for the rollup node manager.
    pub fn event_listener(&mut self) -> EventStream<RollupManagerEvent> {
        if let Some(event_sender) = &self.event_sender {
            return event_sender.new_listener();
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
            self.chain.handle_block_from_peer(block_with_peer.clone());

            // TODO: remove this once we deprecate l2geth.
            // Store the block signature in the database
            let db = self.database.clone();
            let block_hash = block_with_peer.block.hash_slow();
            let signature = block_with_peer.signature;
            tokio::spawn(async move {
                let tx = if let Ok(tx) = db.tx_mut().await {
                    tx
                } else {
                    tracing::warn!(target: "scroll::node::manager", %block_hash, sig=%signature, "Failed to create database transaction");
                    return;
                };
                if let Err(err) = tx.insert_signature(block_hash, signature).await {
                    tracing::warn!(
                        target: "scroll::node::manager",
                        %block_hash, sig=%signature, error=?err,
                        "Failed to store block signature; execution client already persisted the block"
                    );
                } else {
                    tracing::trace!(
                        target: "scroll::node::manager",
                        %block_hash, sig=%signature,
                        "Persisted block signature to database"
                    );
                }
                if let Err(err) = tx.commit().await {
                    tracing::warn!(target: "scroll::node::manager", %block_hash, sig=%signature, error=?err, "Failed to commit database transaction");
                }
            });
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

    /// Handles a chain orchestrator event.
    fn handle_chain_orchestrator_event(&mut self, event: ChainOrchestratorEvent) {
        trace!(target: "scroll::node::manager", ?event, "Received chain orchestrator event");

        if let Some(event_sender) = self.event_sender.as_ref() {
            event_sender.notify(RollupManagerEvent::ChainOrchestratorEvent(event.clone()));
        }

        match event {
            #[allow(clippy::match_same_arms)]
            ChainOrchestratorEvent::BatchCommitIndexed { .. } => {
                // Uncomment once we implement issue #273.
                // // if we detected a batch revert event, we reset the pipeline and the engine
                // driver. if let Some(new_safe_head) = safe_head {
                //     self.derivation_pipeline.handle_batch_revert(batch_info.index);
                //     self.engine.clear_l1_payload_attributes();
                //     self.engine.set_head_block_info(new_safe_head);
                //     self.engine.set_safe_block_info(new_safe_head);
                // }
                // // push the batch info into the derivation pipeline.
                // self.derivation_pipeline.push_batch(batch_info, l1_block_number);
            }
            ChainOrchestratorEvent::BatchFinalized(block_number, finalized_batches) => {
                // Uncomment once we implement issue #273.
                // // update the fcs on new finalized block.
                // if let Some(finalized_block) = finalized_block {
                //     self.engine.set_finalized_block_info(finalized_block);
                // }
                // Remove once we implement issue #273.
                // Update the derivation pipeline on new finalized batch.
                for batch_info in finalized_batches {
                    self.metrics.handle_finalized_batch_index.set(batch_info.index as f64);
                    self.derivation_pipeline.push_batch(batch_info, block_number);
                }
            }
            ChainOrchestratorEvent::L1BlockFinalized(l1_block_number, finalized_batches, ..) => {
                self.metrics.handle_l1_finalized_block_number.set(l1_block_number as f64);
                // update the sequencer's l1 finalized block number.
                if let Some(sequencer) = self.sequencer.as_mut() {
                    sequencer.set_l1_finalized_block_number(l1_block_number);
                }
                // Uncomment once we implement issue #273.
                // // update the fcs on new finalized block.
                // if let Some(finalized_block) = finalized_block {
                //     self.engine.set_finalized_block_info(finalized_block);
                // }
                // Remove once we implement issue #273.
                // push all finalized batches into the derivation pipeline.
                for batch_info in finalized_batches {
                    self.derivation_pipeline.push_batch(batch_info, l1_block_number);
                }
            }
            ChainOrchestratorEvent::L1Reorg {
                l1_block_number,
                queue_index,
                l2_head_block_info,
                l2_safe_block_info,
            } => {
                self.metrics.handle_l1_reorg_l1_block_number.set(l1_block_number as f64);
                self.metrics
                    .handle_l1_reorg_l2_head_block_number
                    .set(l2_head_block_info.as_ref().map_or(0, |info| info.number) as f64);
                self.metrics
                    .handle_l1_reorg_l2_safe_block_number
                    .set(l2_safe_block_info.as_ref().map_or(0, |info| info.number) as f64);

                // Handle the reorg in the engine driver.
                self.engine.handle_l1_reorg(
                    l1_block_number,
                    l2_head_block_info,
                    l2_safe_block_info,
                );

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
            ChainOrchestratorEvent::ChainExtended(chain_import) |
            ChainOrchestratorEvent::ChainReorged(chain_import) => {
                self.metrics
                    .handle_chain_import_block_number
                    .set(chain_import.chain.last().unwrap().number as f64);

                // Issue the new chain to the engine driver for processing.
                self.engine.handle_chain_import(chain_import)
            }
            ChainOrchestratorEvent::OptimisticSync(block) => {
                let block_info: BlockInfo = (&block).into();

                self.metrics.handle_optimistic_syncing_block_number.set(block_info.number as f64);

                // Issue the new block info to the engine driver for processing.
                self.engine.handle_optimistic_sync(block_info)
            }
            _ => {}
        }
    }

    /// Handles a chain orchestrator error.
    fn handle_chain_orchestrator_error(&self, err: &ChainOrchestratorError) {
        error!(
            target: "scroll::node::manager",
            error = ?err,
            msg = %err,
            "Error occurred in the chain orchestrator"
        );

        match err {
            ChainOrchestratorError::L1MessageMismatch { expected, actual } => {
                counter!(
                    "manager_handle_chain_orchestrator_event_failed",
                    "type" => "l1_message_mismatch",
                )
                .increment(1);

                if let Some(event_sender) = self.event_sender.as_ref() {
                    event_sender.notify(RollupManagerEvent::L1MessageConsolidationError {
                        expected: *expected,
                        actual: *actual,
                    });
                }
            }
            ChainOrchestratorError::DatabaseError(DatabaseError::L1MessageNotFound(start)) => {
                counter!(
                    "manager_handle_chain_orchestrator_event_failed",
                    "type" => "l1_message_not_found",
                )
                .increment(1);

                if let Some(event_sender) = self.event_sender.as_ref() {
                    event_sender.notify(RollupManagerEvent::L1MessageMissingInDatabase {
                        start: start.clone(),
                    });
                }
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
                    if let Some(sequencer) = self.sequencer.as_mut() {
                        sequencer.handle_new_payload(&block);
                    }
                    if let Some(event_sender) = self.event_sender.as_ref() {
                        event_sender.notify(RollupManagerEvent::BlockImported(block.clone()));
                    }
                    self.chain.consolidate_validated_l2_blocks(vec![(&block).into()]);
                }
                self.network.handle().block_import_outcome(outcome);
            }
            EngineDriverEvent::NewPayload(payload) => {
                if let Some(signer) = self.signer.as_mut() {
                    let _ = signer.sign_block(payload.clone()).inspect_err(|err| error!(target: "scroll::node::manager", ?err, "Failed to send new payload to signer"));
                }

                self.sequencer
                    .as_mut()
                    .expect("Sequencer must be enabled to build payload")
                    .handle_new_payload(&payload);

                if let Some(event_sender) = self.event_sender.as_ref() {
                    event_sender.notify(RollupManagerEvent::BlockSequenced(payload));
                }
            }
            EngineDriverEvent::L1BlockConsolidated(consolidation_outcome) => {
                self.chain.persist_l1_consolidated_blocks(
                    vec![consolidation_outcome.block_info().clone()],
                    *consolidation_outcome.batch_info(),
                );

                if let Some(event_sender) = self.event_sender.as_ref() {
                    event_sender.notify(RollupManagerEvent::L1DerivedBlockConsolidated(
                        consolidation_outcome,
                    ));
                }
            }
            EngineDriverEvent::ChainImportOutcome(outcome) => {
                if let Some(block) = outcome.outcome.block() {
                    if let Some(sequencer) = self.sequencer.as_mut() {
                        sequencer.handle_new_payload(&block);
                    }
                    if let Some(event_sender) = self.event_sender.as_ref() {
                        event_sender.notify(RollupManagerEvent::BlockImported(block));
                    }
                    self.chain.consolidate_validated_l2_blocks(
                        outcome.chain.iter().map(|b| b.into()).collect(),
                    );
                }
                self.network.handle().block_import_outcome(outcome.outcome);
            }
        }
    }

    /// Handles an [`L1Notification`] from the L1 watcher.
    fn handle_l1_notification(&mut self, notification: L1Notification) {
        self.metrics.handle_l1_notification.increment(1);
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

    /// Returns true if the manager has capacity to accept new L1 notifications.
    pub fn has_capacity_for_l1_notifications(&self) -> bool {
        let chain_orchestrator_has_capacity = self.chain.pending_futures_len() <
            CHAIN_ORCHESTRATOR_MAX_PENDING_FUTURES - L1_NOTIFICATION_CHANNEL_BUDGET as usize;
        let engine_has_capacity = self.engine.pending_futures_len() < ENGINE_MAX_PENDING_FUTURES;
        let derivation_pipeline_has_capacity =
            self.derivation_pipeline.batch_queue_size() < DERIVATION_PIPELINE_MAX_PENDING_BATCHES;
        chain_orchestrator_has_capacity && engine_has_capacity && derivation_pipeline_has_capacity
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
            this.metrics.handle_rollup_manager_command.increment(1);
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
                RollupManagerCommand::NetworkHandle(tx) => {
                    let network_handle = this.network.handle();
                    tx.send(network_handle.clone())
                        .expect("Failed to send network handle to handle");
                }
                RollupManagerCommand::EnableAutomaticSequencing(tx) => {
                    let success = if let Some(block_time) = this.block_time_config {
                        if this.block_building_trigger.is_none() {
                            this.block_building_trigger = Some(delayed_interval(block_time));
                            info!(target: "scroll::node::manager", "Enabled automatic sequencing with interval {}ms", block_time);
                        } else {
                            info!(target: "scroll::node::manager", "Automatic sequencing already enabled");
                        }
                        true
                    } else {
                        warn!(target: "scroll::node::manager", "Cannot enable automatic sequencing: sequencer and block time not configured");
                        false
                    };
                    tx.send(success).expect("Failed to send enable automatic sequencing response");
                }
                RollupManagerCommand::DisableAutomaticSequencing(tx) => {
                    let was_enabled = this.block_building_trigger.is_some();
                    this.block_building_trigger = None;
                    info!(target: "scroll::node::manager", "Disabled automatic sequencing (was enabled: {})", was_enabled);
                    tx.send(true).expect("Failed to send disable automatic sequencing response");
                }
            }
        }

        // Drain all EngineDriver events.
        while let Poll::Ready(Some(event)) = this.engine.poll_next_unpin(cx) {
            this.metrics.handle_engine_driver_event.increment(1);
            this.handle_engine_driver_event(event);
        }

        proceed_if!(
            en_synced,
            // Handle new block production.
            if let Some(Poll::Ready(Some(attributes))) =
                this.sequencer.as_mut().map(|x| x.poll_next_unpin(cx))
            {
                this.metrics.handle_new_block_produced.increment(1);
                this.engine.handle_build_new_payload(attributes);
            }
        );

        let mut maybe_more_l1_rx_events = false;
        proceed_if!(
            en_synced && this.has_capacity_for_l1_notifications(),
            maybe_more_l1_rx_events = poll_nested_stream_with_budget!(
                "l1_notification_rx",
                "L1Notification channel",
                L1_NOTIFICATION_CHANNEL_BUDGET,
                this.l1_notification_rx
                    .as_mut()
                    .map(|rx| rx.poll_next_unpin(cx))
                    .unwrap_or(Poll::Ready(None)),
                |event: Arc<L1Notification>| this.handle_l1_notification((*event).clone()),
            )
        );

        // Drain all chain orchestrator events.
        while let Poll::Ready(Some(result)) = this.chain.poll_next_unpin(cx) {
            this.metrics.handle_chain_orchestrator_event.increment(1);
            match result {
                Ok(event) => this.handle_chain_orchestrator_event(event),
                Err(err) => {
                    this.handle_chain_orchestrator_error(&err);
                }
            }
        }

        // Drain all signer events.
        while let Some(Poll::Ready(Some(event))) =
            this.signer.as_mut().map(|s| s.poll_next_unpin(cx))
        {
            this.metrics.handle_signer_event.increment(1);
            match event {
                SignerEvent::SignedBlock { block, signature } => {
                    trace!(target: "scroll::node::manager", ?block, ?signature, "Received signed block from signer, announcing to the network");
                    // Send SignerEvent for test monitoring
                    if let Some(event_sender) = this.event_sender.as_ref() {
                        event_sender.notify(RollupManagerEvent::SignerEvent(
                            SignerEvent::SignedBlock { block: block.clone(), signature },
                        ));
                    }

                    // TODO: remove this once we deprecate l2geth.
                    // Store the block signature in the database
                    let db = this.database.clone();
                    let block_hash = block.hash_slow();
                    tokio::spawn(async move {
                        let tx = if let Ok(tx) = db.tx_mut().await {
                            tx
                        } else {
                            tracing::warn!(target: "scroll::node::manager", %block_hash, sig=%signature, "Failed to create database transaction");
                            return;
                        };
                        if let Err(err) = tx.insert_signature(block_hash, signature).await {
                            tracing::warn!(
                                target: "scroll::node::manager",
                                %block_hash, sig=%signature, error=?err,
                                "Failed to store block signature; execution client already persisted the block"
                            );
                        } else {
                            tracing::trace!(
                                target: "scroll::node::manager",
                                %block_hash, sig=%signature,
                                "Persisted block signature to database"
                            );
                        }
                        if let Err(err) = tx.commit().await {
                            tracing::warn!(target: "scroll::node::manager", %block_hash, sig=%signature, error=?err, "Failed to commit database transaction");
                        }
                    });

                    this.chain.handle_sequenced_block(NewBlockWithPeer {
                        peer_id: Default::default(),
                        block: block.clone(),
                        signature,
                    });
                    this.network.handle().announce_block(block, signature);
                }
            }
        }

        proceed_if!(
            en_synced,
            // Check if we need to trigger the build of a new payload.
            if let (Some(Poll::Ready(_)), Some(sequencer)) = (
                this.block_building_trigger.as_mut().map(|trigger| trigger.poll_tick(cx)),
                this.sequencer.as_mut()
            ) {
                this.metrics.handle_build_new_payload.increment(1);
                if !this.consensus.should_sequence_block(
                    this.signer
                        .as_ref()
                        .map(|s| &s.address)
                        .expect("signer must be set if sequencer is present"),
                ) {
                    trace!(target: "scroll::node::manager", "Signer is not authorized to sequence block for this slot");
                } else if this.engine.is_payload_building_in_progress() {
                    warn!(target: "scroll::node::manager", "Payload building is already in progress skipping slot");
                } else {
                    sequencer.build_payload_attributes();
                }
            }
        );

        // Poll Derivation Pipeline and push attribute in queue if any.
        while let Poll::Ready(Some(attributes)) = this.derivation_pipeline.poll_next_unpin(cx) {
            this.metrics.handle_l1_consolidation.increment(1);
            this.engine.handle_l1_consolidation(attributes)
        }

        // Handle network manager events.
        while let Poll::Ready(Some(event)) = this.network.poll_next_unpin(cx) {
            this.metrics.handle_network_manager_event.increment(1);
            this.handle_network_manager_event(event);
        }

        if maybe_more_l1_rx_events {
            cx.waker().wake_by_ref();
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
