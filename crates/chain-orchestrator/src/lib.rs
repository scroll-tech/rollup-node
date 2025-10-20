//! A library responsible for orchestrating the L2 chain based on data received from L1 and over the
//! L2 p2p network.

use alloy_eips::Encodable2718;
use alloy_primitives::{b256, bytes::Bytes, keccak256, B256};
use alloy_provider::Provider;
use alloy_rpc_types_engine::ExecutionPayloadV1;
use futures::StreamExt;
use reth_chainspec::EthChainSpec;
use reth_network_api::{BlockDownloaderProvider, FullNetwork};
use reth_network_p2p::FullBlockClient;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_scroll_primitives::ScrollBlock;
use reth_tasks::shutdown::Shutdown;
use reth_tokio_util::{EventSender, EventStream};
use rollup_node_primitives::{
    BatchCommitData, BatchInfo, BlockConsolidationOutcome, BlockInfo, ChainImport,
    L1MessageEnvelope, L2BlockInfoWithL1Messages,
};
use rollup_node_providers::L1MessageProvider;
use rollup_node_sequencer::{Sequencer, SequencerEvent};
use rollup_node_signer::{SignatureAsBytes, SignerEvent, SignerHandle};
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_network::Scroll;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_db::{
    Database, DatabaseError, DatabaseReadOperations, DatabaseWriteOperations, L1MessageKey,
    UnwindResult,
};
use scroll_derivation_pipeline::{BatchDerivationResult, DerivationPipeline};
use scroll_engine::Engine;
use scroll_network::{
    BlockImportOutcome, NewBlockWithPeer, ScrollNetwork, ScrollNetworkManagerEvent,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Instant,
    vec,
};
use strum::IntoEnumIterator;
use tokio::sync::mpsc::{self, Receiver, UnboundedReceiver};

mod config;
pub use config::ChainOrchestratorConfig;

mod consensus;
pub use consensus::{Consensus, NoopConsensus, SystemContractConsensus};

mod consolidation;
use consolidation::reconcile_batch;

mod event;
pub use event::ChainOrchestratorEvent;

mod error;
pub use error::ChainOrchestratorError;

mod handle;
pub use handle::{ChainOrchestratorCommand, ChainOrchestratorHandle, DatabaseQuery};

mod metrics;
pub use metrics::{ChainOrchestratorItem, ChainOrchestratorMetrics};

mod sync;
pub use sync::{SyncMode, SyncState};

mod status;
pub use status::ChainOrchestratorStatus;

use crate::consolidation::BlockConsolidationAction;

/// The mask used to mask the L1 message queue hash.
const L1_MESSAGE_QUEUE_HASH_MASK: B256 =
    b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000");

/// The number of headers to fetch in each request when fetching headers from peers.
const HEADER_FETCH_COUNT: u64 = 100;

/// The size of the event channel used to broadcast events to listeners.
const EVENT_CHANNEL_SIZE: usize = 5000;

/// The [`ChainOrchestrator`] is responsible for orchestrating the progression of the L2 chain
/// based on data consolidated from L1 and the data received over the p2p network.
#[derive(Debug)]
pub struct ChainOrchestrator<
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
    ChainSpec,
    L1MP,
    L2P,
    EC,
> {
    /// The configuration for the chain orchestrator.
    config: ChainOrchestratorConfig<ChainSpec>,
    /// The receiver for commands sent to the chain orchestrator.
    handle_rx: UnboundedReceiver<ChainOrchestratorCommand<N>>,
    /// The `BlockClient` that is used to fetch blocks from peers over p2p.
    block_client: Arc<FullBlockClient<<N as BlockDownloaderProvider>::Client>>,
    /// The L2 client that is used to interact with the L2 chain.
    l2_client: Arc<L2P>,
    /// The reference to database.
    database: Arc<Database>,
    /// The metrics for the chain orchestrator.
    metrics: HashMap<ChainOrchestratorItem, ChainOrchestratorMetrics>,
    /// The current sync state of the [`ChainOrchestrator`].
    sync_state: SyncState,
    /// A receiver for [`L1Notification`]s from the [`rollup_node_watcher::L1Watcher`].
    l1_notification_rx: Receiver<Arc<L1Notification>>,
    /// The network manager that manages the scroll p2p network.
    network: ScrollNetwork<N>,
    /// The consensus algorithm used by the rollup node.
    consensus: Box<dyn Consensus + 'static>,
    /// The engine used to communicate with the execution layer.
    engine: Engine<EC>,
    /// The sequencer used to build blocks.
    sequencer: Option<Sequencer<L1MP, ChainSpec>>,
    /// The signer used to sign messages.
    signer: Option<SignerHandle>,
    /// The derivation pipeline used to derive L2 blocks from batches.
    derivation_pipeline: DerivationPipeline,
    /// Optional event sender for broadcasting events to listeners.
    event_sender: Option<EventSender<ChainOrchestratorEvent>>,
}

impl<
        N: FullNetwork<Primitives = ScrollNetworkPrimitives> + Send + Sync + 'static,
        ChainSpec: ScrollHardforks + EthChainSpec + Send + Sync + 'static,
        L1MP: L1MessageProvider + Unpin + Clone + Send + Sync + 'static,
        L2P: Provider<Scroll> + 'static,
        EC: ScrollEngineApi + Sync + Send + 'static,
    > ChainOrchestrator<N, ChainSpec, L1MP, L2P, EC>
{
    /// Creates a new chain orchestrator.
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        database: Arc<Database>,
        config: ChainOrchestratorConfig<ChainSpec>,
        block_client: Arc<FullBlockClient<<N as BlockDownloaderProvider>::Client>>,
        l2_provider: L2P,
        l1_notification_rx: Receiver<Arc<L1Notification>>,
        network: ScrollNetwork<N>,
        consensus: Box<dyn Consensus + 'static>,
        engine: Engine<EC>,
        sequencer: Option<Sequencer<L1MP, ChainSpec>>,
        signer: Option<SignerHandle>,
        derivation_pipeline: DerivationPipeline,
    ) -> Result<(Self, ChainOrchestratorHandle<N>), ChainOrchestratorError> {
        let (handle_tx, handle_rx) = mpsc::unbounded_channel();
        let handle = ChainOrchestratorHandle::new(handle_tx);
        Ok((
            Self {
                block_client,
                l2_client: Arc::new(l2_provider),
                database,
                config,
                metrics: ChainOrchestratorItem::iter()
                    .map(|i| {
                        let label = i.as_str();
                        (i, ChainOrchestratorMetrics::new_with_labels(&[("item", label)]))
                    })
                    .collect(),
                sync_state: SyncState::default(),
                l1_notification_rx,
                network,
                consensus,
                engine,
                sequencer,
                signer,
                derivation_pipeline,
                handle_rx,
                event_sender: None,
            },
            handle,
        ))
    }

    /// Drives the [`ChainOrchestrator`] future until a [`Shutdown`] signal is received.
    pub async fn run_until_shutdown(mut self, mut shutdown: Shutdown) {
        loop {
            tokio::select! {
                biased;

                _guard = &mut shutdown => {
                    break;
                }
                Some(command) = self.handle_rx.recv() => {
                    if let Err(err) = self.handle_command(command).await {
                        tracing::error!(target: "scroll::chain_orchestrator", ?err, "Error handling command");
                    }
                }
                Some(event) = async {
                    if let Some(event) = self.signer.as_mut() {
                        event.next().await
                    } else {
                        unreachable!()
                    }
                }, if self.signer.is_some() => {
                    let res = self.handle_signer_event(event).await;
                    self.handle_outcome(res);
                }
                Some(event) = async {
                    if let Some(seq) = self.sequencer.as_mut() {
                        seq.next().await
                    } else {
                        unreachable!()
                    }
                }, if self.sequencer.is_some() && self.sync_state.is_synced() => {
                    let res = self.handle_sequencer_event(event).await;
                    self.handle_outcome(res);
                }
                Some(batch) = self.derivation_pipeline.next() => {
                    let res = self.handle_derived_batch(batch).await;
                    self.handle_outcome(res);
                }
                Some(event) = self.network.events().next() => {
                    let res = self.handle_network_event(event).await;
                    self.handle_outcome(res);
                }
                Some(notification) = self.l1_notification_rx.recv(), if self.sync_state.l2().is_synced() && self.derivation_pipeline.is_empty() => {
                    let res = self.handle_l1_notification(notification).await;
                    self.handle_outcome(res);
                }

            }
        }
    }

    /// Handles the outcome of an operation, logging errors and notifying event listeners as
    /// appropriate.
    fn handle_outcome(
        &self,
        outcome: Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError>,
    ) {
        match outcome {
            Ok(Some(event)) => self.notify(event),
            Err(err) => {
                tracing::error!(target: "scroll::chain_orchestrator", ?err, "Encountered error in the chain orchestrator");
            }
            Ok(None) => {}
        }
    }

    /// Handles an event from the signer.
    async fn handle_signer_event(
        &self,
        event: SignerEvent,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        tracing::info!(target: "scroll::chain_orchestrator", ?event, "Handling signer event");
        match event {
            SignerEvent::SignedBlock { block, signature } => {
                let hash = block.hash_slow();
                self.database
                    .tx_mut(move |tx| async move {
                        tx.set_l2_head_block_number(block.header.number).await?;
                        tx.insert_signature(hash, signature).await
                    })
                    .await?;
                self.network.handle().announce_block(block.clone(), signature);
                Ok(Some(ChainOrchestratorEvent::SignedBlock { block, signature }))
            }
        }
    }

    /// Handles an event from the sequencer.
    async fn handle_sequencer_event(
        &mut self,
        event: rollup_node_sequencer::SequencerEvent,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        tracing::info!(target: "scroll::chain_orchestrator", ?event, "Handling sequencer event");
        match event {
            SequencerEvent::NewSlot => {
                if self.consensus.should_sequence_block(
                    self.signer
                        .as_ref()
                        .map(|s| &s.address)
                        .expect("signer must be set if sequencer is present"),
                ) {
                    self.sequencer
                        .as_mut()
                        .expect("sequencer must be present")
                        .start_payload_building(&mut self.engine)
                        .await?;
                }
            }
            SequencerEvent::PayloadReady(payload_id) => {
                if let Some(block) = self
                    .sequencer
                    .as_mut()
                    .expect("sequencer must be present")
                    .finalize_payload_building(payload_id, &mut self.engine)
                    .await?
                {
                    let block_info: L2BlockInfoWithL1Messages = (&block).into();
                    self.database
                        .update_l1_messages_from_l2_blocks(vec![block_info.clone()])
                        .await?;
                    self.signer
                        .as_mut()
                        .expect("signer must be present")
                        .sign_block(block.clone())?;
                    return Ok(Some(ChainOrchestratorEvent::BlockSequenced(block)));
                }
            }
        }

        Ok(None)
    }

    /// Handles a command sent to the chain orchestrator.
    async fn handle_command(
        &mut self,
        command: ChainOrchestratorCommand<N>,
    ) -> Result<(), ChainOrchestratorError> {
        tracing::debug!(target: "scroll::chain_orchestrator", ?command, "Handling command");
        match command {
            ChainOrchestratorCommand::BuildBlock => {
                if let Some(sequencer) = self.sequencer.as_mut() {
                    sequencer.start_payload_building(&mut self.engine).await?;
                } else {
                    tracing::error!(target: "scroll::chain_orchestrator", "Received BuildBlock command but sequencer is not configured");
                }
            }
            ChainOrchestratorCommand::EventListener(tx) => {
                let _ = tx.send(self.event_listener());
            }
            ChainOrchestratorCommand::Status(tx) => {
                let (l1_latest, l1_finalized, l1_processed) = self
                    .database
                    .tx(|tx| async move {
                        let l1_latest = tx.get_latest_l1_block_number().await?;
                        let l1_finalized = tx.get_finalized_l1_block_number().await?;
                        let l1_processed = tx.get_processed_l1_block_number().await?;
                        Ok::<_, ChainOrchestratorError>((l1_latest, l1_finalized, l1_processed))
                    })
                    .await?;
                let status = ChainOrchestratorStatus::new(
                    &self.sync_state,
                    l1_latest,
                    l1_finalized,
                    l1_processed,
                    self.engine.fcs().clone(),
                );
                let _ = tx.send(status);
            }
            ChainOrchestratorCommand::NetworkHandle(tx) => {
                let _ = tx.send(self.network.handle().clone());
            }
            ChainOrchestratorCommand::UpdateFcsHead((head, sender)) => {
                self.engine.update_fcs(Some(head), None, None).await?;
                self.database
                    .tx_mut(move |tx| async move {
                        tx.purge_l1_message_to_l2_block_mappings(Some(head.number + 1)).await?;
                        tx.set_l2_head_block_number(head.number).await
                    })
                    .await?;
                self.notify(ChainOrchestratorEvent::FcsHeadUpdated(head));
                let _ = sender.send(());
            }
            ChainOrchestratorCommand::EnableAutomaticSequencing(tx) => {
                if let Some(sequencer) = self.sequencer.as_mut() {
                    sequencer.enable();
                    let _ = tx.send(true);
                } else {
                    tracing::error!(target: "scroll::chain_orchestrator", "Received EnableAutomaticSequencing command but sequencer is not configured");
                    let _ = tx.send(false);
                }
            }
            ChainOrchestratorCommand::DisableAutomaticSequencing(tx) => {
                if let Some(sequencer) = self.sequencer.as_mut() {
                    sequencer.disable();
                    let _ = tx.send(true);
                } else {
                    tracing::error!(target: "scroll::chain_orchestrator", "Received DisableAutomaticSequencing command but sequencer is not configured");
                    let _ = tx.send(false);
                }
            }
            ChainOrchestratorCommand::DatabaseQuery(query) => match query {
                DatabaseQuery::GetL1MessageByIndex(index, sender) => {
                    let tx = self.database.tx().await?;
                    let l1_message = tx.get_l1_message_by_index(index).await?;
                    let _ = sender.send(l1_message);
                }
            },
            #[cfg(feature = "test-utils")]
            ChainOrchestratorCommand::SetGossip((enabled, tx)) => {
                self.network.handle().set_gossip(enabled).await;
                let _ = tx.send(());
            }
        }

        Ok(())
    }

    /// Returns a new event listener for the rollup node manager.
    pub fn event_listener(&mut self) -> EventStream<ChainOrchestratorEvent> {
        if let Some(event_sender) = &self.event_sender {
            return event_sender.new_listener();
        };

        let event_sender = EventSender::new(EVENT_CHANNEL_SIZE);
        let event_listener = event_sender.new_listener();
        self.event_sender = Some(event_sender);

        event_listener
    }

    /// Notifies all event listeners of the given event.
    fn notify(&self, event: ChainOrchestratorEvent) {
        if let Some(s) = self.event_sender.as_ref() {
            s.notify(event);
        }
    }

    /// Handles a derived batch by inserting the derived blocks into the database.
    async fn handle_derived_batch(
        &mut self,
        batch: BatchDerivationResult,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let batch_info = batch.batch_info;
        tracing::info!(target: "scroll::chain_orchestrator", batch_info = ?batch_info, num_blocks = batch.attributes.len(), "Handling derived batch");

        let batch_reconciliation_result =
            reconcile_batch(&self.l2_client, batch, self.engine.fcs()).await?;
        let aggregated_actions = batch_reconciliation_result.aggregate_actions();

        let mut reorg_results = vec![];
        for action in aggregated_actions.actions {
            let outcome = match action {
                BlockConsolidationAction::Skip(_) => {
                    unreachable!("Skip actions have been filtered out in aggregation")
                }
                BlockConsolidationAction::UpdateSafeHead(block_info) => {
                    tracing::info!(target: "scroll::chain_orchestrator", ?block_info, "Updating safe head to consolidated block");
                    self.engine
                        .update_fcs(None, Some(block_info.block_info), Some(block_info.block_info))
                        .await?;
                    BlockConsolidationOutcome::UpdateFcs(block_info)
                }
                BlockConsolidationAction::Reorg(attributes) => {
                    tracing::info!(target: "scroll::chain_orchestrator", block_number = ?attributes.block_number, "Reorging chain to derived block");
                    // We reorg the head to the safe block and then build the payload for the
                    // attributes.
                    let head = *self.engine.fcs().safe_block_info();
                    if head.number != attributes.block_number - 1 {
                        return Err(ChainOrchestratorError::InvalidBatchReorg {
                            batch_info,
                            safe_block_number: head.number,
                            derived_block_number: attributes.block_number,
                        });
                    }
                    let fcu = self.engine.build_payload(Some(head), attributes.attributes).await?;
                    let payload = self
                        .engine
                        .get_payload(fcu.payload_id.expect("payload_id can not be None"))
                        .await?;

                    let block_info: L2BlockInfoWithL1Messages = (&payload)
                        .try_into()
                        .map_err(ChainOrchestratorError::RollupNodePrimitiveError)?;
                    let result = self.engine.new_payload(payload).await?;
                    if result.is_invalid() {
                        return Err(ChainOrchestratorError::InvalidBatch(
                            block_info.block_info,
                            batch_info,
                        ));
                    }

                    // Update the forkchoice state to the new head.
                    self.engine
                        .update_fcs(
                            Some(block_info.block_info),
                            Some(block_info.block_info),
                            Some(block_info.block_info),
                        )
                        .await?;

                    reorg_results.push(block_info.clone());
                    BlockConsolidationOutcome::Reorged(block_info)
                }
            };

            self.notify(ChainOrchestratorEvent::BlockConsolidated(outcome.clone()));
        }

        let batch_consolidation_outcome =
            batch_reconciliation_result.into_batch_consolidation_outcome(reorg_results).await?;

        // Insert the batch consolidation outcome into the database.
        let consolidation_outcome = batch_consolidation_outcome.clone();
        self.database.insert_batch_consolidation_outcome(consolidation_outcome).await?;

        Ok(Some(ChainOrchestratorEvent::BatchConsolidated(batch_consolidation_outcome)))
    }

    /// Handles an L1 notification.
    async fn handle_l1_notification(
        &mut self,
        notification: Arc<L1Notification>,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        match &*notification {
            L1Notification::Processed(block_number) => {
                let block_number = *block_number;
                self.database.set_processed_l1_block_number(block_number).await?;
                Ok(None)
            }
            L1Notification::Reorg(block_number) => self.handle_l1_reorg(*block_number).await,
            L1Notification::Consensus(update) => {
                self.consensus.update_config(update);
                Ok(None)
            }
            L1Notification::NewBlock(block_number) => self.handle_l1_new_block(*block_number).await,
            L1Notification::Finalized(block_number) => {
                self.handle_l1_finalized(*block_number).await
            }
            L1Notification::BatchCommit(batch) => self.handle_batch_commit(batch.clone()).await,
            L1Notification::L1Message { message, block_number, block_timestamp: _ } => {
                self.handle_l1_message(message.clone(), *block_number).await
            }
            L1Notification::Synced => {
                tracing::info!(target: "scroll::chain_orchestrator", "L1 is now synced");
                self.sync_state.l1_mut().set_synced();
                if self.sync_state.is_synced() {
                    self.consolidate_chain().await?;
                }
                self.notify(ChainOrchestratorEvent::L1Synced);
                Ok(None)
            }
            L1Notification::BatchFinalization { hash: _hash, index, block_number } => {
                self.handle_l1_batch_finalization(*index, *block_number).await
            }
        }
    }

    async fn handle_l1_new_block(
        &self,
        block_number: u64,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        self.database.set_latest_l1_block_number(block_number).await?;
        Ok(Some(ChainOrchestratorEvent::NewL1Block(block_number)))
    }

    /// Handles a reorganization event by deleting all indexed data which is greater than the
    /// provided block number.
    async fn handle_l1_reorg(
        &mut self,
        block_number: u64,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let metric = self.metrics.get(&ChainOrchestratorItem::L1Reorg).expect("metric exists");
        let now = Instant::now();
        let genesis_hash = self.config.chain_spec().genesis_hash();
        let UnwindResult { l1_block_number, queue_index, l2_head_block_number, l2_safe_block_info } =
            self.database.unwind(genesis_hash, block_number).await?;

        let l2_head_block_info = if let Some(block_number) = l2_head_block_number {
            // Fetch the block hash of the new L2 head block.
            let block_hash = self
                .l2_client
                .get_block_by_number(block_number.into())
                .full()
                .await?
                .expect("L2 head block must exist")
                .header
                .hash_slow();

            // Cancel the inflight payload building job if the head has changed.
            if let Some(s) = self.sequencer.as_mut() {
                s.cancel_payload_building_job();
            };

            Some(BlockInfo { number: block_number, hash: block_hash })
        } else {
            None
        };

        // If the L1 reorg is before the origin of the inflight payload building job, cancel it.
        if Some(l1_block_number) <
            self.sequencer
                .as_ref()
                .and_then(|s| s.payload_building_job().map(|p| p.l1_origin()))
                .flatten()
        {
            if let Some(s) = self.sequencer.as_mut() {
                s.cancel_payload_building_job();
            };
        }

        // TODO: Add retry logic
        if l2_head_block_info.is_some() || l2_safe_block_info.is_some() {
            self.engine.update_fcs(l2_head_block_info, l2_safe_block_info, None).await?;
        }

        metric.task_duration.record(now.elapsed().as_secs_f64());

        let event = ChainOrchestratorEvent::L1Reorg {
            l1_block_number,
            queue_index,
            l2_head_block_info,
            l2_safe_block_info,
        };

        Ok(Some(event))
    }

    /// Handles a finalized event by updating the chain orchestrator L1 finalized block, returning
    /// the new finalized L2 chain block and the list of finalized batches.
    async fn handle_l1_finalized(
        &mut self,
        block_number: u64,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let metric =
            self.metrics.get(&ChainOrchestratorItem::L1Finalization).expect("metric exists");
        let now = Instant::now();

        let finalized_batches = self
            .database
            .tx_mut(move |tx| async move {
                // Set the latest finalized L1 block in the database.
                tx.set_finalized_l1_block_number(block_number).await?;

                // Get all unprocessed batches that have been finalized by this L1 block
                // finalization.
                tx.fetch_and_update_unprocessed_finalized_batches(block_number).await
            })
            .await?;

        for batch in &finalized_batches {
            self.derivation_pipeline.push_batch(Arc::new(*batch)).await;
        }

        metric.task_duration.record(now.elapsed().as_secs_f64());

        Ok(Some(ChainOrchestratorEvent::L1BlockFinalized(block_number, finalized_batches)))
    }

    /// Handles a batch input by inserting it into the database.
    async fn handle_batch_commit(
        &self,
        batch: BatchCommitData,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let metric = self.metrics.get(&ChainOrchestratorItem::BatchCommit).expect("metric exists");
        let now = Instant::now();

        let event = self
            .database
            .tx_mut(move |tx| {
                let batch_clone = batch.clone();
                async move {
                    let prev_batch_index = batch_clone.index - 1;

                    // Perform a consistency check to ensure the previous commit batch exists in the
                    // database.
                    if tx.get_batch_by_index(prev_batch_index).await?.is_none() {
                        return Err(ChainOrchestratorError::BatchCommitGap(batch_clone.index));
                    }

                    // remove any batches with an index greater than the previous batch.
                    let affected = tx.delete_batches_gt_batch_index(prev_batch_index).await?;

                    // handle the case of a batch revert.
                    let new_safe_head = if affected > 0 {
                        tx.delete_l2_blocks_gt_batch_index(prev_batch_index).await?;
                        tx.get_highest_block_for_batch_index(prev_batch_index).await?
                    } else {
                        None
                    };

                    let event = ChainOrchestratorEvent::BatchCommitIndexed {
                        batch_info: BatchInfo::new(batch_clone.index, batch_clone.hash),
                        l1_block_number: batch_clone.block_number,
                        safe_head: new_safe_head,
                    };

                    // insert the batch and commit the transaction.
                    tx.insert_batch(batch_clone).await?;
                    Ok::<_, ChainOrchestratorError>(Some(event))
                }
            })
            .await?;

        metric.task_duration.record(now.elapsed().as_secs_f64());

        Ok(event)
    }

    /// Handles a batch finalization event by updating the batch input in the database.
    async fn handle_l1_batch_finalization(
        &mut self,
        batch_index: u64,
        block_number: u64,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let event = self
            .database
            .tx_mut(move |tx| async move {
                // finalize all batches up to `batch_index`.
                tx.finalize_batches_up_to_index(batch_index, block_number).await?;

                // Get all unprocessed batches that have been finalized by this L1 block
                // finalization.
                let finalized_block_number = tx.get_finalized_l1_block_number().await?;
                if finalized_block_number >= block_number {
                    let finalized_batches = tx
                        .fetch_and_update_unprocessed_finalized_batches(finalized_block_number)
                        .await?;

                    return Ok(Some(ChainOrchestratorEvent::BatchFinalized(
                        block_number,
                        finalized_batches,
                    )));
                }

                Ok::<_, ChainOrchestratorError>(None)
            })
            .await;

        if let Ok(Some(ChainOrchestratorEvent::BatchFinalized(_, batches))) = &event {
            for batch in batches {
                self.derivation_pipeline.push_batch(Arc::new(*batch)).await;
            }
        }

        event
    }

    /// Handles an L1 message by inserting it into the database.
    async fn handle_l1_message(
        &self,
        l1_message: TxL1Message,
        l1_block_number: u64,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let metric = self.metrics.get(&ChainOrchestratorItem::L1Message).expect("metric exists");
        let now = Instant::now();

        let event = ChainOrchestratorEvent::L1MessageCommitted(l1_message.queue_index);
        let queue_hash = compute_l1_message_queue_hash(
            &self.database,
            &l1_message,
            self.config.l1_v2_message_queue_start_index(),
        )
        .await?;
        let l1_message = L1MessageEnvelope::new(l1_message, l1_block_number, None, queue_hash);

        // Perform a consistency check to ensure the previous L1 message exists in the database.
        self.database
            .tx_mut(move |tx| {
                let l1_message = l1_message.clone();
                async move {
                    if l1_message.transaction.queue_index > 0 &&
                        tx.get_n_l1_messages(
                            Some(L1MessageKey::from_queue_index(
                                l1_message.transaction.queue_index - 1,
                            )),
                            1,
                        )
                        .await?
                        .is_empty()
                    {
                        return Err(ChainOrchestratorError::L1MessageQueueGap(
                            l1_message.transaction.queue_index,
                        ));
                    }

                    tx.insert_l1_message(l1_message.clone()).await?;
                    Ok::<_, ChainOrchestratorError>(())
                }
            })
            .await?;

        metric.task_duration.record(now.elapsed().as_secs_f64());

        Ok(Some(event))
    }

    // /// Wraps a pending chain orchestrator future, metering the completion of it.
    // pub fn handle_metered(
    //     &mut self,
    //     item: ChainOrchestratorItem,
    //     chain_orchestrator_fut: PendingChainOrchestratorFuture,
    // ) -> PendingChainOrchestratorFuture {
    //     let metric = self.metrics.get(&item).expect("metric exists").clone();
    //     let fut_wrapper = Box::pin(async move {
    //         let now = Instant::now();
    //         let res = chain_orchestrator_fut.await;
    //         metric.task_duration.record(now.elapsed().as_secs_f64());
    //         res
    //     });
    //     fut_wrapper
    // }

    async fn handle_network_event(
        &mut self,
        event: ScrollNetworkManagerEvent,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        match event {
            ScrollNetworkManagerEvent::NewBlock(block_with_peer) => {
                self.notify(ChainOrchestratorEvent::NewBlockReceived(block_with_peer.clone()));
                Ok(self.handle_block_from_peer(block_with_peer).await?)
            }
        }
    }

    /// Handles a new block received from a peer.
    async fn handle_block_from_peer(
        &mut self,
        block_with_peer: NewBlockWithPeer,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        tracing::debug!(target: "scroll::chain_orchestrator", block_hash = ?block_with_peer.block.header.hash_slow(), block_number = ?block_with_peer.block.number, peer_id = ?block_with_peer.peer_id, "Received new block from peer");

        if let Err(err) =
            self.consensus.validate_new_block(&block_with_peer.block, &block_with_peer.signature)
        {
            tracing::error!(target: "scroll::node::manager", ?err, "consensus checks failed on block {:?} from peer {:?}", block_with_peer.block.hash_slow(), block_with_peer.peer_id);
            self.network.handle().block_import_outcome(BlockImportOutcome {
                peer: block_with_peer.peer_id,
                result: Err(err.into()),
            });

            return Ok(Some(ChainOrchestratorEvent::BlockFailedConsensusChecks(
                block_with_peer.block.header.hash_slow(),
                block_with_peer.peer_id,
            )));
        }

        // We optimistically persist the signature upon passing consensus checks.
        let block_hash = block_with_peer.block.header.hash_slow();
        self.database.insert_signature(block_hash, block_with_peer.signature).await?;

        let received_block_number = block_with_peer.block.number;
        let received_block_hash = block_with_peer.block.header.hash_slow();
        let current_head_block_number = self.engine.fcs().head_block_info().number;
        let current_head_block_hash = self.engine.fcs().head_block_info().hash;
        let current_safe_block_number = self.engine.fcs().safe_block_info().number;

        // If the received block number has a block number greater than the current head by more
        // than the optimistic sync threshold, we optimistically sync the chain.
        if received_block_number >
            current_head_block_number + self.config.optimistic_sync_threshold()
        {
            tracing::trace!(target: "scroll::chain_orchestrator", ?received_block_number, ?current_head_block_number, "Received new block from peer with block number greater than current head by more than the optimistic sync threshold");
            let block_info = BlockInfo {
                number: received_block_number,
                hash: block_with_peer.block.header.hash_slow(),
            };
            self.engine.optimistic_sync(block_info).await?;
            self.sync_state.l2_mut().set_syncing();

            // Purge all L1 message to L2 block mappings as they may be invalid after an
            // optimistic sync.
            self.database.purge_l1_message_to_l2_block_mappings(None).await?;

            return Ok(Some(ChainOrchestratorEvent::OptimisticSync(block_info)));
        }

        // If the block number is greater than the current head we attempt to extend the chain.
        let mut new_headers = if received_block_number > current_head_block_number {
            // Fetch the headers for the received block until we can reconcile it with the current
            // chain head.
            let fetch_count = received_block_number - current_head_block_number;
            let new_headers = if received_block_number > current_head_block_number + 1 {
                tracing::trace!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, ?current_head_block_number, fetch_count, "Fetching headers to extend chain");
                self.block_client
                    .get_full_block_range(received_block_hash, fetch_count)
                    .await
                    .into_iter()
                    .rev()
                    .map(|b| b.into_block())
                    .collect()
            } else {
                vec![block_with_peer.block.clone()]
            };

            // If the first header in the new headers has a parent hash that matches the current
            // head hash, we can import the chain.
            if new_headers.first().expect("at least one header exists").parent_hash ==
                current_head_block_hash
            {
                tracing::trace!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, "Received block from peer that extends the current head");
                let chain_import = self.import_chain(new_headers, block_with_peer).await?;
                return Ok(Some(ChainOrchestratorEvent::ChainExtended(chain_import)));
            }

            VecDeque::from(new_headers)
        } else {
            // If the block is less than or equal to the current head check if we already have it in
            // the chain.
            let current_chain_block = self
                .l2_client
                .get_block_by_number(received_block_number.into())
                .full()
                .await?
                .ok_or(ChainOrchestratorError::L2BlockNotFoundInL2Client(received_block_number))?;

            if current_chain_block.header.hash_slow() == received_block_hash {
                tracing::info!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, "Received block from peer that is already in the chain");
                return Ok(Some(ChainOrchestratorEvent::BlockAlreadyKnown(
                    received_block_hash,
                    block_with_peer.peer_id,
                )));
            }

            // Assert that we are not reorging below the safe head.
            let current_safe_info = self.engine.fcs().safe_block_info();
            if received_block_number <= current_safe_info.number {
                tracing::warn!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, current_safe_info = ?self.engine.fcs().safe_block_info(), "Received block from peer that would reorg below the safe head - ignoring");
                return Err(ChainOrchestratorError::L2SafeBlockReorgDetected);
            }

            // Check to assert that we have received a newer chain.
            let current_head = self
                .l2_client
                .get_block_by_number(current_head_block_number.into())
                .full()
                .await?
                .expect("current head block must exist");

            // If the timestamp of the received block is less than or equal to the current head,
            // we ignore it.
            if block_with_peer.block.header.timestamp <= current_head.header.timestamp {
                tracing::debug!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, current_head_hash = ?current_head.header.hash_slow(), current_head_number = current_head_block_number, "Received block from peer that is older than the current head - ignoring");
                return Ok(Some(ChainOrchestratorEvent::OldForkReceived {
                    headers: vec![block_with_peer.block.header],
                    peer_id: block_with_peer.peer_id,
                    signature: block_with_peer.signature,
                }));
            }

            // Check if the parent hash of the received block is in the chain.
            let parent_block = self
                .l2_client
                .get_block_by_hash(block_with_peer.block.header.parent_hash)
                .full()
                .await?;
            if let Some(parent_block) = parent_block {
                // If the parent block has a block number equal to or greater than the current safe
                // head then it is safe to reorg.
                if parent_block.header.number >= current_safe_block_number {
                    tracing::debug!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, "Received block from peer that extends an earlier part of the chain");
                    let chain_import = self
                        .import_chain(vec![block_with_peer.block.clone()], block_with_peer)
                        .await?;
                    return Ok(Some(ChainOrchestratorEvent::ChainReorged(chain_import)));
                }
                // If the parent block has a block number less than the current safe head then would
                // suggest a reorg of the safe head - reject it.
                tracing::warn!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, current_safe_info = ?self.engine.fcs().safe_block_info(), "Received block from peer that would reorg below the safe head - ignoring");
                return Err(ChainOrchestratorError::L2SafeBlockReorgDetected);
            }

            VecDeque::from([block_with_peer.block.clone()])
        };

        // If we reach this point, we have a block that is not in the current chain and does not
        // extend the current head. This implies a reorg. We attempt to reconcile the fork.
        while current_safe_block_number + 1 <
            new_headers.front().expect("at least one header exists").number
        {
            let parent_hash = new_headers.front().expect("at least one header exists").parent_hash;
            let parent_number = new_headers.front().expect("at least one header exists").number - 1;
            let fetch_count = HEADER_FETCH_COUNT.min(parent_number - current_safe_block_number);
            tracing::trace!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, ?parent_hash, ?parent_number, %current_safe_block_number, fetch_count, "Fetching headers to find common ancestor for fork");
            let headers: Vec<ScrollBlock> = self
                .block_client
                .get_full_block_range(parent_hash, fetch_count)
                .await
                .into_iter()
                .map(|b| b.into_block())
                .collect();

            let mut index = None;
            for (i, header) in headers.iter().enumerate() {
                let current_block = self
                    .l2_client
                    .get_block_by_number(header.number.into())
                    .full()
                    .await?
                    .expect("block must exist")
                    .into_consensus()
                    .map_transactions(|tx| tx.inner.into_inner());

                if header.hash_slow() == current_block.header.hash_slow() {
                    index = Some(i);
                    break;
                }
            }

            if let Some(index) = index {
                tracing::trace!(target: "scroll::chain_orchestrator", ?received_block_hash, ?received_block_number, common_ancestor = ?headers[index].hash_slow(), common_ancestor_number = headers[index].number, "Found common ancestor for fork - reorging to new chain");
                for header in headers.into_iter().take(index) {
                    new_headers.push_front(header);
                }
                let chain_import = self.import_chain(new_headers.into(), block_with_peer).await?;
                return Ok(Some(ChainOrchestratorEvent::ChainReorged(chain_import)));
            };

            // If we did not find a common ancestor, we add all the fetched headers to the front of
            // the deque and continue fetching.
            for header in headers {
                new_headers.push_front(header);
            }
        }

        Err(ChainOrchestratorError::L2SafeBlockReorgDetected)
    }

    /// Imports a chain of headers into the L2 chain.
    async fn import_chain(
        &mut self,
        chain: Vec<ScrollBlock>,
        block_with_peer: NewBlockWithPeer,
    ) -> Result<ChainImport, ChainOrchestratorError> {
        let chain_head_hash = chain.last().expect("at least one header exists").hash_slow();
        let chain_head_number = chain.last().expect("at least one header exists").number;
        tracing::info!(target: "scroll::chain_orchestrator", num_blocks = chain.len(), ?chain_head_hash, ?chain_head_number, "Received chain from peer");

        // If we are in consolidated mode, validate the L1 messages in the new blocks.
        if self.sync_state.is_synced() {
            self.validate_l1_messages(&chain).await?;
        }

        // Validate the new blocks by sending them to the engine.
        for block in &chain {
            let payload = ExecutionPayloadV1::from_block_slow(block);
            let status = self.engine.new_payload(payload).await?;
            tracing::debug!(target: "scroll::chain_orchestrator", block_number = block.number, block_hash = ?block.hash_slow(), ?status, "New payload status from engine");

            if status.is_invalid() {
                tracing::warn!(target: "scroll::chain_orchestrator", block_number = block.number, block_hash = ?block.hash_slow(), ?status, "Received invalid block from peer");
                self.network.handle().block_import_outcome(BlockImportOutcome::invalid_block(
                    block_with_peer.peer_id,
                ));
                return Err(ChainOrchestratorError::InvalidBlock);
            }
        }

        // Update the FCS to the new head.
        let result = self
            .engine
            .update_fcs(
                Some(BlockInfo { number: chain_head_number, hash: chain_head_hash }),
                None,
                None,
            )
            .await?;

        // If the FCS update resulted in an invalid state, we return an error.
        if result.is_invalid() {
            tracing::warn!(target: "scroll::chain_orchestrator", ?chain_head_hash, ?chain_head_number, ?result, "Failed to update FCS after importing new chain from peer");
            return Err(ChainOrchestratorError::InvalidBlock);
        }

        // If we were previously in L2 syncing mode and the FCS update resulted in a valid state, we
        // transition the L2 sync state to synced and consolidate the chain.
        if result.is_valid() && self.sync_state.l2().is_syncing() {
            tracing::info!(target: "scroll::chain_orchestrator", "L2 is now synced");
            self.sync_state.l2_mut().set_synced();

            // If both L1 and L2 are now synced, we transition to consolidated mode by consolidating
            // the chain.
            if self.sync_state.is_synced() {
                self.consolidate_chain().await?;
            }
        }

        // Persist the L1 message to L2 block mappings for reorg awareness, update the l2 head block
        // number and handle the valid block import if we are in a synced state and the
        // result is valid.
        if self.sync_state.is_synced() && result.is_valid() {
            let blocks = chain.iter().map(|block| block.into()).collect::<Vec<_>>();
            self.database
                .tx_mut(move |tx| {
                    let blocks = blocks.clone();
                    async move {
                        tx.update_l1_messages_from_l2_blocks(blocks).await?;
                        tx.set_l2_head_block_number(block_with_peer.block.header.number).await
                    }
                })
                .await?;

            self.network.handle().block_import_outcome(BlockImportOutcome::valid_block(
                block_with_peer.peer_id,
                block_with_peer.block,
                Bytes::copy_from_slice(&block_with_peer.signature.sig_as_bytes()),
            ));
        }

        Ok(ChainImport {
            chain,
            peer_id: block_with_peer.peer_id,
            signature: block_with_peer.signature,
        })
    }

    /// Consolidates the chain by validating all unsafe blocks from the current safe head to the
    /// current head.
    ///
    /// This involves validating the L1 messages in the blocks against the expected L1 messages
    /// synced from L1.
    async fn consolidate_chain(&self) -> Result<(), ChainOrchestratorError> {
        tracing::trace!(target: "scroll::chain_orchestrator", fcs = ?self.engine.fcs(), "Consolidating chain from safe to head");

        let safe_block_number = self.engine.fcs().safe_block_info().number;
        let head_block_number = self.engine.fcs().head_block_info().number;

        if head_block_number == safe_block_number {
            tracing::trace!(target: "scroll::chain_orchestrator", "No unsafe blocks to consolidate");

            self.notify(ChainOrchestratorEvent::ChainConsolidated {
                from: safe_block_number,
                to: head_block_number,
            });
            return Ok(());
        }

        let start_block_number = safe_block_number + 1;
        // TODO: Make fetching parallel but ensure concurrency limits are respected.
        let mut blocks_to_validate = vec![];
        for block_number in start_block_number..=head_block_number {
            let block = self
                .l2_client
                .get_block_by_number(block_number.into())
                .full()
                .await?
                .ok_or(ChainOrchestratorError::L2BlockNotFoundInL2Client(block_number))?
                .into_consensus()
                .map_transactions(|tx| tx.inner.into_inner());
            blocks_to_validate.push(block);
        }

        self.validate_l1_messages(&blocks_to_validate).await?;

        self.database
            .update_l1_messages_from_l2_blocks(
                blocks_to_validate.into_iter().map(|b| (&b).into()).collect(),
            )
            .await?;

        self.notify(ChainOrchestratorEvent::ChainConsolidated {
            from: safe_block_number,
            to: head_block_number,
        });

        Ok(())
    }

    /// Validates the L1 messages in the provided blocks against the expected L1 messages synced
    /// from L1.
    async fn validate_l1_messages(
        &self,
        blocks: &[ScrollBlock],
    ) -> Result<(), ChainOrchestratorError> {
        let l1_message_hashes = blocks
            .iter()
            .flat_map(|block| {
                // Get the L1 messages from the block body.
                block
                    .body
                    .transactions()
                    .filter(|&tx| tx.is_l1_message())
                    // The hash for L1 messages is the trie hash of the transaction.
                    .map(|tx| tx.trie_hash())
                    .collect::<Vec<B256>>()
            })
            .collect::<Vec<B256>>();

        // No L1 messages in the blocks, nothing to validate.
        if l1_message_hashes.is_empty() {
            return Ok(());
        }

        let first_block_number =
            blocks.first().expect("at least one block exists because we have l1 messages").number;
        let count = l1_message_hashes.len();
        let mut database_messages = self
            .database
            .get_n_l1_messages(Some(L1MessageKey::block_number(first_block_number)), count)
            .await?
            .into_iter();

        for message_hash in l1_message_hashes {
            // Get the expected L1 message from the database.
            let expected_hash = database_messages
                .next()
                .map(|m| m.transaction.tx_hash())
                .ok_or(ChainOrchestratorError::L1MessageNotFound(L1MessageKey::TransactionHash(
                    message_hash,
                )))
                .inspect_err(|_| {
                    self.notify(ChainOrchestratorEvent::L1MessageNotFoundInDatabase(
                        L1MessageKey::TransactionHash(message_hash),
                    ));
                })?;

            // If the received and expected L1 messages do not match return an error.
            if message_hash != expected_hash {
                self.notify(ChainOrchestratorEvent::L1MessageMismatch {
                    expected: expected_hash,
                    actual: message_hash,
                });
                return Err(ChainOrchestratorError::L1MessageMismatch {
                    expected: expected_hash,
                    actual: message_hash,
                });
            }
        }

        Ok(())
    }
}

/// Computes the queue hash by taking the previous queue hash and performing a 2-to-1 hash with the
/// current transaction hash using keccak. It then applies a mask to the last 32 bits as these bits
/// are used to store the timestamp at which the message was enqueued in the contract. For the first
/// message in the queue, the previous queue hash is zero. If the L1 message queue index is before
/// migration to `L1MessageQueueV2`, the queue hash will be None.
///
/// The solidity contract (`L1MessageQueueV2.sol`) implementation is defined here: <https://github.com/scroll-tech/scroll-contracts/blob/67c1bde19c1d3462abf8c175916a2bb3c89530e4/src/L1/rollup/L1MessageQueueV2.sol#L379-L403>
async fn compute_l1_message_queue_hash(
    database: &Arc<Database>,
    l1_message: &TxL1Message,
    l1_v2_message_queue_start_index: u64,
) -> Result<Option<alloy_primitives::FixedBytes<32>>, ChainOrchestratorError> {
    let queue_hash = if l1_message.queue_index == l1_v2_message_queue_start_index {
        let mut input = B256::default().to_vec();
        input.append(&mut l1_message.tx_hash().to_vec());
        Some(keccak256(input) & L1_MESSAGE_QUEUE_HASH_MASK)
    } else if l1_message.queue_index > l1_v2_message_queue_start_index {
        let index = l1_message.queue_index - 1;
        let mut input = database
            .get_n_l1_messages(Some(L1MessageKey::from_queue_index(index)), 1)
            .await?
            .first()
            .map(|m| m.queue_hash)
            .ok_or(DatabaseError::L1MessageNotFound(L1MessageKey::QueueIndex(index)))?
            .unwrap_or_default()
            .to_vec();

        input.append(&mut l1_message.tx_hash().to_vec());
        Some(keccak256(input) & L1_MESSAGE_QUEUE_HASH_MASK)
    } else {
        None
    };
    Ok(queue_hash)
}

// #[cfg(test)]
// mod test {
//     use std::vec;

//     use super::*;
//     use alloy_consensus::Header;
//     use alloy_eips::{BlockHashOrNumber, BlockNumHash};
//     use alloy_primitives::{address, bytes, B256, U256};
//     use alloy_provider::{ProviderBuilder, RootProvider};
//     use alloy_transport::mock::Asserter;
//     use arbitrary::{Arbitrary, Unstructured};
//     use futures::StreamExt;
//     use parking_lot::Mutex;
//     use rand::Rng;
//     use reth_eth_wire_types::HeadersDirection;
//     use reth_network_api::BlockClient;
//     use reth_network_p2p::{
//         download::DownloadClient,
//         error::PeerRequestResult,
//         headers::client::{HeadersClient, HeadersRequest},
//         priority::Priority,
//         BodiesClient,
//     };
//     use reth_network_peers::{PeerId, WithPeerId};
//     use reth_primitives_traits::Block;
//     use reth_scroll_chainspec::{ScrollChainSpec, SCROLL_MAINNET};
//     use rollup_node_primitives::BatchCommitData;
//     use scroll_alloy_network::Scroll;
//     use scroll_db::test_utils::setup_test_db;
//     use std::{collections::HashMap, ops::RangeInclusive, sync::Arc};

//     type ScrollBody = <ScrollBlock as Block>::Body;

//     const TEST_OPTIMISTIC_SYNC_THRESHOLD: u64 = 100;
//     const TEST_CHAIN_BUFFER_SIZE: usize = 2000;
//     const TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY: u64 = 953885;

//     /// A headers+bodies client that stores the headers and bodies in memory, with an artificial
//     /// soft bodies response limit that is set to 20 by default.
//     ///
//     /// This full block client can be [Clone]d and shared between multiple tasks.
//     #[derive(Clone, Debug)]
//     struct TestScrollFullBlockClient {
//         headers: Arc<Mutex<HashMap<B256, Header>>>,
//         bodies: Arc<Mutex<HashMap<B256, <ScrollBlock as Block>::Body>>>,
//         // soft response limit, max number of bodies to respond with
//         soft_limit: usize,
//     }

//     impl Default for TestScrollFullBlockClient {
//         fn default() -> Self {
//             let mainnet_genesis: reth_scroll_primitives::ScrollBlock =
//                 serde_json::from_str(include_str!("../testdata/genesis_block.json")).unwrap();
//             let (header, body) = mainnet_genesis.split();
//             let hash = header.hash_slow();
//             let headers = HashMap::from([(hash, header)]);
//             let bodies = HashMap::from([(hash, body)]);
//             Self {
//                 headers: Arc::new(Mutex::new(headers)),
//                 bodies: Arc::new(Mutex::new(bodies)),
//                 soft_limit: 20,
//             }
//         }
//     }

//     impl DownloadClient for TestScrollFullBlockClient {
//         /// Reports a bad message from a specific peer.
//         fn report_bad_message(&self, _peer_id: PeerId) {}

//         /// Retrieves the number of connected peers.
//         ///
//         /// Returns the number of connected peers in the test scenario (1).
//         fn num_connected_peers(&self) -> usize {
//             1
//         }
//     }

//     /// Implements the `HeadersClient` trait for the `TestFullBlockClient` struct.
//     impl HeadersClient for TestScrollFullBlockClient {
//         type Header = Header;
//         /// Specifies the associated output type.
//         type Output = futures::future::Ready<PeerRequestResult<Vec<Header>>>;

//         /// Retrieves headers with a given priority level.
//         ///
//         /// # Arguments
//         ///
//         /// * `request` - A `HeadersRequest` indicating the headers to retrieve.
//         /// * `_priority` - A `Priority` level for the request.
//         ///
//         /// # Returns
//         ///
//         /// A `Ready` future containing a `PeerRequestResult` with a vector of retrieved headers.
//         fn get_headers_with_priority(
//             &self,
//             request: HeadersRequest,
//             _priority: Priority,
//         ) -> Self::Output {
//             let headers = self.headers.lock();

//             // Initializes the block hash or number.
//             let mut block: BlockHashOrNumber = match request.start {
//                 BlockHashOrNumber::Hash(hash) => headers.get(&hash).cloned(),
//                 BlockHashOrNumber::Number(num) => {
//                     headers.values().find(|h| h.number == num).cloned()
//                 }
//             }
//             .map(|h| h.number.into())
//             .unwrap();

//             // Retrieves headers based on the provided limit and request direction.
//             let resp = (0..request.limit)
//                 .filter_map(|_| {
//                     headers.iter().find_map(|(hash, header)| {
//                         // Checks if the header matches the specified block or number.
//                         BlockNumHash::new(header.number,
// *hash).matches_block_or_num(&block).then(                             || {
//                                 match request.direction {
//                                     HeadersDirection::Falling => block =
// header.parent_hash.into(),                                     HeadersDirection::Rising => block
// = (header.number + 1).into(),                                 }
//                                 header.clone()
//                             },
//                         )
//                     })
//                 })
//                 .collect::<Vec<_>>();

//             // Returns a future containing the retrieved headers with a random peer ID.
//             futures::future::ready(Ok(WithPeerId::new(PeerId::random(), resp)))
//         }
//     }

//     /// Implements the `BodiesClient` trait for the `TestFullBlockClient` struct.
//     impl BodiesClient for TestScrollFullBlockClient {
//         type Body = ScrollBody;
//         /// Defines the output type of the function.
//         type Output = futures::future::Ready<PeerRequestResult<Vec<Self::Body>>>;

//         /// Retrieves block bodies corresponding to provided hashes with a given priority.
//         ///
//         /// # Arguments
//         ///
//         /// * `hashes` - A vector of block hashes to retrieve bodies for.
//         /// * `_priority` - Priority level for block body retrieval (unused in this
// implementation).         ///
//         /// # Returns
//         ///
//         /// A future containing the result of the block body retrieval operation.
//         fn get_block_bodies_with_priority_and_range_hint(
//             &self,
//             hashes: Vec<B256>,
//             _priority: Priority,
//             _range_hint: Option<RangeInclusive<u64>>,
//         ) -> Self::Output {
//             // Acquire a lock on the bodies.
//             let bodies = self.bodies.lock();

//             // Create a future that immediately returns the result of the block body retrieval
//             // operation.
//             futures::future::ready(Ok(WithPeerId::new(
//                 PeerId::random(),
//                 hashes
//                     .iter()
//                     .filter_map(|hash| bodies.get(hash).cloned())
//                     .take(self.soft_limit)
//                     .collect(),
//             )))
//         }
//     }

//     impl BlockClient for TestScrollFullBlockClient {
//         type Block = ScrollBlock;
//     }

//     async fn setup_test_chain_orchestrator() -> (
//         ChainOrchestrator<
//             ScrollChainSpec,
//             TestScrollFullBlockClient,
//             RootProvider,
//             RootProvider<Scroll>,
//         >,
//         Arc<Database>,
//     ) {
//         // Get a provider to the node.
//         // TODO: update to use a real node URL.
//         let assertor = Asserter::new();
//         let mainnet_genesis: <Scroll as scroll_alloy_network::Network>::BlockResponse =
//             serde_json::from_str(include_str!("../testdata/genesis_block_rpc.json"))
//                 .expect("Failed to parse mainnet genesis block");
//         assertor.push_success(&mainnet_genesis);
//         let provider = ProviderBuilder::<_, _,
// Scroll>::default().connect_mocked_client(assertor);

//         let db = Arc::new(setup_test_db().await);
//         (
//             ChainOrchestrator::new(
//                 db.clone(),
//                 SCROLL_MAINNET.clone(),
//                 TestScrollFullBlockClient::default(),
//                 provider,
//                 TEST_OPTIMISTIC_SYNC_THRESHOLD,
//                 TEST_CHAIN_BUFFER_SIZE,
//                 TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY,
//             )
//             .await
//             .unwrap(),
//             db,
//         )
//     }

//     #[tokio::test]
//     async fn test_handle_commit_batch() {
//         // Instantiate chain orchestrator and db
//         let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

//         // Generate unstructured bytes.
//         let mut bytes = [0u8; 1024];
//         rand::rng().fill(bytes.as_mut_slice());
//         let mut u = Unstructured::new(&bytes);

//         // Insert a batch commit in the database to satisfy the chain orchestrator consistency
//         // checks
//         let batch_0 = BatchCommitData { index: 0, ..Arbitrary::arbitrary(&mut u).unwrap() };
//         let tx = db.tx_mut().await.unwrap();
//         tx.insert_batch(batch_0).await.unwrap();
//         tx.commit().await.unwrap();

//         let batch_1 = BatchCommitData { index: 1, ..Arbitrary::arbitrary(&mut u).unwrap() };
//         chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(batch_1.clone()));

//         let event = chain_orchestrator.next().await.unwrap().unwrap();

//         // Verify the event structure
//         match event {
//             ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
//                 assert_eq!(batch_info.index, batch_1.index);
//                 assert_eq!(batch_info.hash, batch_1.hash);
//                 assert_eq!(safe_head, None); // No safe head since no batch revert
//             }
//             _ => panic!("Expected BatchCommitIndexed event"),
//         }

//         let tx = db.tx().await.unwrap();
//         let batch_commit_result = tx.get_batch_by_index(batch_1.index).await.unwrap().unwrap();
//         assert_eq!(batch_1, batch_commit_result);
//     }

//     #[tokio::test]
//     async fn test_handle_batch_commit_with_revert() {
//         // Instantiate chain orchestrator and db
//         let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

//         // Generate unstructured bytes.
//         let mut bytes = [0u8; 1024];
//         rand::rng().fill(bytes.as_mut_slice());
//         let mut u = Unstructured::new(&bytes);

//         // Insert batch 0 into the database to satisfy the consistency conditions in the chain
//         // orchestrator
//         let batch_0 = BatchCommitData {
//             index: 99,
//             calldata: Arc::new(vec![].into()),
//             ..Arbitrary::arbitrary(&mut u).unwrap()
//         };
//         let tx = db.tx_mut().await.unwrap();
//         tx.insert_batch(batch_0).await.unwrap();
//         tx.commit().await.unwrap();

//         // Create sequential batches
//         let batch_1 = BatchCommitData {
//             index: 100,
//             calldata: Arc::new(vec![].into()),
//             ..Arbitrary::arbitrary(&mut u).unwrap()
//         };
//         let batch_2 = BatchCommitData {
//             index: 101,
//             calldata: Arc::new(vec![].into()),
//             ..Arbitrary::arbitrary(&mut u).unwrap()
//         };
//         let batch_3 = BatchCommitData {
//             index: 102,
//             calldata: Arc::new(vec![].into()),
//             ..Arbitrary::arbitrary(&mut u).unwrap()
//         };

//         // Index first batch
//         chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(batch_1.clone()));
//         let event = chain_orchestrator.next().await.unwrap().unwrap();
//         match event {
//             ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
//                 assert_eq!(batch_info.index, 100);
//                 assert_eq!(safe_head, None);
//             }
//             _ => panic!("Expected BatchCommitIndexed event"),
//         }

//         // Index second batch
//         chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(batch_2.clone()));
//         let event = chain_orchestrator.next().await.unwrap().unwrap();
//         match event {
//             ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
//                 assert_eq!(batch_info.index, 101);
//                 assert_eq!(safe_head, None);
//             }
//             _ => panic!("Expected BatchCommitIndexed event"),
//         }

//         // Index third batch
//         chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(batch_3.clone()));
//         let event = chain_orchestrator.next().await.unwrap().unwrap();
//         match event {
//             ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
//                 assert_eq!(batch_info.index, 102);
//                 assert_eq!(safe_head, None);
//             }
//             _ => panic!("Expected BatchCommitIndexed event"),
//         }

//         // Add some L2 blocks for the batches
//         let batch_1_info = BatchInfo::new(batch_1.index, batch_1.hash);
//         let batch_2_info = BatchInfo::new(batch_2.index, batch_2.hash);

//         let block_1 = L2BlockInfoWithL1Messages {
//             block_info: BlockInfo { number: 500, hash: Arbitrary::arbitrary(&mut u).unwrap() },
//             l1_messages: vec![],
//         };
//         let block_2 = L2BlockInfoWithL1Messages {
//             block_info: BlockInfo { number: 501, hash: Arbitrary::arbitrary(&mut u).unwrap() },
//             l1_messages: vec![],
//         };
//         let block_3 = L2BlockInfoWithL1Messages {
//             block_info: BlockInfo { number: 502, hash: Arbitrary::arbitrary(&mut u).unwrap() },
//             l1_messages: vec![],
//         };

//         chain_orchestrator.persist_l1_consolidated_blocks(vec![block_1.clone()], batch_1_info);
//         chain_orchestrator.next().await.unwrap().unwrap();

//         chain_orchestrator.persist_l1_consolidated_blocks(vec![block_2.clone()], batch_2_info);
//         chain_orchestrator.next().await.unwrap().unwrap();

//         chain_orchestrator.persist_l1_consolidated_blocks(vec![block_3.clone()], batch_2_info);
//         chain_orchestrator.next().await.unwrap().unwrap();

//         // Now simulate a batch revert by submitting a new batch with index 101
//         // This should delete batch 102 and any blocks associated with it
//         let new_batch_2 = BatchCommitData {
//             index: 101,
//             calldata: Arc::new(vec![1, 2, 3].into()), // Different data
//             ..Arbitrary::arbitrary(&mut u).unwrap()
//         };

//         chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(new_batch_2.
// clone()));         let event = chain_orchestrator.next().await.unwrap().unwrap();

//         // Verify the event indicates a batch revert
//         match event {
//             ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
//                 assert_eq!(batch_info.index, 101);
//                 assert_eq!(batch_info.hash, new_batch_2.hash);
//                 // Safe head should be the highest block from batch index <= 100
//                 assert_eq!(safe_head, Some(block_1.block_info));
//             }
//             _ => panic!("Expected BatchCommitIndexed event"),
//         }

//         // Verify batch 102 was deleted
//         let tx = db.tx().await.unwrap();
//         let batch_102 = tx.get_batch_by_index(102).await.unwrap();
//         assert!(batch_102.is_none());

//         // Verify batch 101 was replaced with new data
//         let updated_batch_101 = tx.get_batch_by_index(101).await.unwrap().unwrap();
//         assert_eq!(updated_batch_101, new_batch_2);

//         // Verify batch 100 still exists
//         let batch_100 = tx.get_batch_by_index(100).await.unwrap();
//         assert!(batch_100.is_some());
//     }

//     #[tokio::test]
//     async fn test_handle_l1_message() {
//         reth_tracing::init_test_tracing();

//         // Instantiate chain orchestrator and db
//         let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

//         // Generate unstructured bytes.
//         let mut bytes = [0u8; 1024];
//         rand::rng().fill(bytes.as_mut_slice());
//         let mut u = Unstructured::new(&bytes);

//         // Insert an initial message in the database to satisfy the consistency checks in the
// chain         //  orchestrator.
//         let message_0 = L1MessageEnvelope {
//             transaction: TxL1Message {
//                 queue_index: TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY - 2,
//                 ..Arbitrary::arbitrary(&mut u).unwrap()
//             },
//             ..Arbitrary::arbitrary(&mut u).unwrap()
//         };
//         let tx = db.tx_mut().await.unwrap();
//         tx.insert_l1_message(message_0).await.unwrap();
//         tx.commit().await.unwrap();

//         let message_1 = TxL1Message {
//             queue_index: TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY - 1,
//             ..Arbitrary::arbitrary(&mut u).unwrap()
//         };
//         let block_number = u64::arbitrary(&mut u).unwrap();
//         chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
//             message: message_1.clone(),
//             block_number,
//             block_timestamp: 0,
//         });

//         let _ = chain_orchestrator.next().await;

//         let tx = db.tx().await.unwrap();
//         let l1_message_result =
//             tx.get_l1_message_by_index(message_1.queue_index).await.unwrap().unwrap();
//         let l1_message = L1MessageEnvelope::new(message_1, block_number, None, None);

//         assert_eq!(l1_message, l1_message_result);
//     }

//     #[tokio::test]
//     async fn test_l1_message_hash_queue() {
//         // Instantiate chain orchestrator and db
//         let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

//         // Insert the previous l1 message in the database to satisfy the chain orchestrator
//         // consistency checks.
//         let message = L1MessageEnvelope {
//             transaction: TxL1Message {
//                 queue_index: TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY - 1,
//                 ..Default::default()
//             },
//             l1_block_number: 1475587,
//             l2_block_number: None,
//             queue_hash: None,
//         };
//         let tx = db.tx_mut().await.unwrap();
//         tx.insert_l1_message(message).await.unwrap();
//         tx.commit().await.unwrap();

//         // insert the previous L1 message in database.
//         chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
//             message: TxL1Message {
//                 queue_index: TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY,
//                 ..Default::default()
//             },
//             block_number: 1475588,
//             block_timestamp: 1745305199,
//         });
//         let _ = chain_orchestrator.next().await.unwrap().unwrap();

//         // <https://sepolia.scrollscan.com/tx/0xd80cd61ac5d8665919da19128cc8c16d3647e1e2e278b931769e986d01c6b910>
//         let message = TxL1Message {
//             queue_index: TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY + 1,
//             gas_limit: 168000,
//             to: address!("Ba50f5340FB9F3Bd074bD638c9BE13eCB36E603d"),
//             value: U256::ZERO,
//             sender: address!("61d8d3E7F7c656493d1d76aAA1a836CEdfCBc27b"),
//             input:
// bytes!("8ef1332e000000000000000000000000323522a8de3cddeddbb67094eecaebc2436d6996000000000000000000000000323522a8de3cddeddbb67094eecaebc2436d699600000000000000000000000000000000000000000000000000038d7ea4c6800000000000000000000000000000000000000000000000000000000000001034de00000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000"
// ),         };
//         chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
//             message: message.clone(),
//             block_number: 14755883,
//             block_timestamp: 1745305200,
//         });

//         let _ = chain_orchestrator.next().await.unwrap().unwrap();

//         let tx = db.tx().await.unwrap();
//         let l1_message_result =
//             tx.get_l1_message_by_index(message.queue_index).await.unwrap().unwrap();

//         assert_eq!(
//             b256!("b2331b9010aac89f012d648fccc1f0a9aa5ef7b7b2afe21be297dd1a00000000"),
//             l1_message_result.queue_hash.unwrap()
//         );
//     }

//     #[tokio::test]
//     async fn test_handle_reorg() {
//         // Instantiate chain orchestrator and db
//         let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

//         // Generate unstructured bytes.
//         let mut bytes = [0u8; 1024];
//         rand::rng().fill(bytes.as_mut_slice());
//         let mut u = Unstructured::new(&bytes);

//         // Insert batch 0 into the database to satisfy the consistency checks in the chain
//         // orchestrator
//         let batch_0 =
//             BatchCommitData { index: 0, block_number: 0, ..Arbitrary::arbitrary(&mut u).unwrap()
// };         let tx = db.tx_mut().await.unwrap();
//         tx.insert_batch(batch_0).await.unwrap();

//         // Insert l1 message into the database to satisfy the consistency checks in the chain
//         // orchestrator
//         let l1_message = L1MessageEnvelope {
//             queue_hash: None,
//             l1_block_number: 0,
//             l2_block_number: None,
//             transaction: TxL1Message { queue_index: 0, ..Arbitrary::arbitrary(&mut u).unwrap() },
//         };
//         tx.insert_l1_message(l1_message).await.unwrap();
//         tx.commit().await.unwrap();

//         // Generate a 3 random batch inputs and set their block numbers
//         let mut batch_commit_block_1 = BatchCommitData::arbitrary(&mut u).unwrap();
//         batch_commit_block_1.block_number = 1;
//         batch_commit_block_1.index = 1;
//         let batch_commit_block_1 = batch_commit_block_1;

//         let mut batch_commit_block_2 = BatchCommitData::arbitrary(&mut u).unwrap();
//         batch_commit_block_2.block_number = 2;
//         batch_commit_block_2.index = 2;
//         let batch_commit_block_2 = batch_commit_block_2;

//         let mut batch_commit_block_3 = BatchCommitData::arbitrary(&mut u).unwrap();
//         batch_commit_block_3.block_number = 3;
//         batch_commit_block_3.index = 3;
//         let batch_commit_block_3 = batch_commit_block_3;

//         // Index batch inputs
//         chain_orchestrator
//             .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
//         chain_orchestrator
//             .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_2.clone()));
//         chain_orchestrator
//             .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_3.clone()));

//         // Generate 3 random L1 messages and set their block numbers
//         let l1_message_block_1 = L1MessageEnvelope {
//             queue_hash: None,
//             l1_block_number: 1,
//             l2_block_number: None,
//             transaction: TxL1Message { queue_index: 1, ..Arbitrary::arbitrary(&mut u).unwrap() },
//         };
//         let l1_message_block_2 = L1MessageEnvelope {
//             queue_hash: None,
//             l1_block_number: 2,
//             l2_block_number: None,
//             transaction: TxL1Message { queue_index: 2, ..Arbitrary::arbitrary(&mut u).unwrap() },
//         };
//         let l1_message_block_3 = L1MessageEnvelope {
//             queue_hash: None,
//             l1_block_number: 3,
//             l2_block_number: None,
//             transaction: TxL1Message { queue_index: 3, ..Arbitrary::arbitrary(&mut u).unwrap() },
//         };

//         // Index L1 messages
//         chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
//             message: l1_message_block_1.clone().transaction,
//             block_number: l1_message_block_1.clone().l1_block_number,
//             block_timestamp: 0,
//         });
//         chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
//             message: l1_message_block_2.clone().transaction,
//             block_number: l1_message_block_2.clone().l1_block_number,
//             block_timestamp: 0,
//         });
//         chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
//             message: l1_message_block_3.clone().transaction,
//             block_number: l1_message_block_3.clone().l1_block_number,
//             block_timestamp: 0,
//         });

//         // Reorg at block 2
//         chain_orchestrator.handle_l1_notification(L1Notification::Reorg(2));

//         for _ in 0..7 {
//             chain_orchestrator.next().await.unwrap().unwrap();
//         }

//         let tx = db.tx().await.unwrap();

//         // Check that the batch input at block 30 is deleted
//         let batch_commits =
//             tx.get_batches().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;

//         assert_eq!(3, batch_commits.len());
//         assert!(batch_commits.contains(&batch_commit_block_1));
//         assert!(batch_commits.contains(&batch_commit_block_2));

//         // check that the L1 message at block 30 is deleted
//         let l1_messages = tx
//             .get_l1_messages(None)
//             .await
//             .unwrap()
//             .map(|res| res.unwrap())
//             .collect::<Vec<_>>()
//             .await;
//         assert_eq!(3, l1_messages.len());
//         assert!(l1_messages.contains(&l1_message_block_1));
//         assert!(l1_messages.contains(&l1_message_block_2));
//     }

//     // We ignore this test for now as it requires a more complex setup which leverages an L2 node
//     // and is already covered in the integration test `can_handle_reorgs_while_sequencing`
//     #[ignore]
//     #[tokio::test]
//     async fn test_handle_reorg_executed_l1_messages() {
//         // Instantiate chain orchestrator and db
//         let (mut chain_orchestrator, _database) = setup_test_chain_orchestrator().await;

//         // Generate unstructured bytes.
//         let mut bytes = [0u8; 8192];
//         rand::rng().fill(bytes.as_mut_slice());
//         let mut u = Unstructured::new(&bytes);

//         // Generate a 3 random batch inputs and set their block numbers
//         let batch_commit_block_1 =
//             BatchCommitData { block_number: 5, index: 5, ..Arbitrary::arbitrary(&mut u).unwrap()
// };         let batch_commit_block_10 = BatchCommitData {
//             block_number: 10,
//             index: 10,
//             ..Arbitrary::arbitrary(&mut u).unwrap()
//         };

//         // Index batch inputs
//         chain_orchestrator
//             .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
//         chain_orchestrator
//             .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_10.clone()));
//         for _ in 0..2 {
//             let _event = chain_orchestrator.next().await.unwrap().unwrap();
//         }

//         let batch_1 = BatchInfo::new(batch_commit_block_1.index, batch_commit_block_1.hash);
//         let batch_10 = BatchInfo::new(batch_commit_block_10.index, batch_commit_block_10.hash);

//         const UNITS_FOR_TESTING: u64 = 20;
//         const L1_MESSAGES_NOT_EXECUTED_COUNT: u64 = 7;
//         let mut l1_messages = Vec::with_capacity(UNITS_FOR_TESTING as usize);
//         for l1_message_queue_index in 0..UNITS_FOR_TESTING {
//             let l1_message = L1MessageEnvelope {
//                 queue_hash: None,
//                 l1_block_number: l1_message_queue_index,
//                 l2_block_number: (UNITS_FOR_TESTING - l1_message_queue_index >
//                     L1_MESSAGES_NOT_EXECUTED_COUNT)
//                     .then_some(l1_message_queue_index),
//                 transaction: TxL1Message {
//                     queue_index: l1_message_queue_index,
//                     ..Arbitrary::arbitrary(&mut u).unwrap()
//                 },
//             };
//             chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
//                 message: l1_message.transaction.clone(),
//                 block_number: l1_message.l1_block_number,
//                 block_timestamp: 0,
//             });
//             chain_orchestrator.next().await.unwrap().unwrap();
//             l1_messages.push(l1_message);
//         }

//         let mut blocks = Vec::with_capacity(UNITS_FOR_TESTING as usize);
//         for block_number in 0..UNITS_FOR_TESTING {
//             let l2_block = L2BlockInfoWithL1Messages {
//                 block_info: BlockInfo {
//                     number: block_number,
//                     hash: Arbitrary::arbitrary(&mut u).unwrap(),
//                 },
//                 l1_messages: (UNITS_FOR_TESTING - block_number > L1_MESSAGES_NOT_EXECUTED_COUNT)
//                     .then_some(vec![l1_messages[block_number as usize].transaction.tx_hash()])
//                     .unwrap_or_default(),
//             };
//             let batch_info = if block_number < 5 {
//                 Some(batch_1)
//             } else if block_number < 10 {
//                 Some(batch_10)
//             } else {
//                 None
//             };
//             if let Some(batch_info) = batch_info {
//                 chain_orchestrator
//                     .persist_l1_consolidated_blocks(vec![l2_block.clone()], batch_info);
//             } else {
//                 chain_orchestrator.consolidate_validated_l2_blocks(vec![l2_block.clone()]);
//             }

//             chain_orchestrator.next().await.unwrap().unwrap();
//             blocks.push(l2_block);
//         }

//         // First we assert that we dont reorg the L2 or message queue hash for a higher block
//         // than any of the L1 messages.
//         chain_orchestrator.handle_l1_notification(L1Notification::Reorg(17));
//         let event = chain_orchestrator.next().await.unwrap().unwrap();
//         assert_eq!(
//             event,
//             ChainOrchestratorEvent::L1Reorg {
//                 l1_block_number: 17,
//                 queue_index: None,
//                 l2_head_block_info: None,
//                 l2_safe_block_info: None
//             }
//         );

//         // Reorg at block 7 which is one of the messages that has not been executed yet. No reorg
//         // but we should ensure the L1 messages have been deleted.
//         chain_orchestrator.handle_l1_notification(L1Notification::Reorg(7));
//         let event = chain_orchestrator.next().await.unwrap().unwrap();

//         assert_eq!(
//             event,
//             ChainOrchestratorEvent::L1Reorg {
//                 l1_block_number: 7,
//                 queue_index: Some(8),
//                 l2_head_block_info: Some(blocks[7].block_info),
//                 l2_safe_block_info: Some(blocks[4].block_info)
//             }
//         );

//         // Now reorg at block 5 which contains L1 messages that have been executed .
//         chain_orchestrator.handle_l1_notification(L1Notification::Reorg(3));
//         let event = chain_orchestrator.next().await.unwrap().unwrap();

//         assert_eq!(
//             event,
//             ChainOrchestratorEvent::L1Reorg {
//                 l1_block_number: 3,
//                 queue_index: Some(4),
//                 l2_head_block_info: Some(blocks[3].block_info),
//                 l2_safe_block_info: Some(BlockInfo::new(
//                     0,
//                     chain_orchestrator.chain_spec.genesis_hash()
//                 )),
//             }
//         );
//     }
// }
