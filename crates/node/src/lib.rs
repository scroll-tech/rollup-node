//! This library contains the main manager for the rollup node.

use alloy_rpc_types_engine::{
    ExecutionPayload, ExecutionPayloadV1, ForkchoiceState as AlloyForkchoiceState,
    PayloadStatusEnum,
};
use futures::{stream::FuturesOrdered, FutureExt, StreamExt};
use reth_tokio_util::{EventSender, EventStream};
use rollup_node_indexer::Indexer;
use rollup_node_watcher::L1Notification;
use scroll_alloy_network::Scroll as ScrollNetwork;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_engine::{EngineDriver, EngineDriverError, ForkchoiceState};
use scroll_network::{
    BlockImportError, BlockImportOutcome, BlockValidation, BlockValidationError, NetworkManager,
    NetworkManagerEvent, NewBlockWithPeer,
};
use scroll_wire::NewBlock;
use std::{
    fmt,
    fmt::Debug,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{Receiver, UnboundedReceiver};
use tokio_stream::wrappers::{ReceiverStream, UnboundedReceiverStream};
use tracing::{error, trace};

pub use event::RollupEvent;
mod event;

pub use consensus::PoAConsensus;
mod consensus;

use consensus::Consensus;
use rollup_node_primitives::BlockInfo;
use rollup_node_providers::{ExecutionPayloadProvider, L1Provider};
use scroll_db::{Database, DatabaseOperations};
use scroll_derivation_pipeline::derive;

/// The size of the event channel.
const EVENT_CHANNEL_SIZE: usize = 100;

/// A future that resolves to a tuple of the block info and the block import outcome.
type PendingBlockImportFuture =
    Pin<Box<dyn Future<Output = (Option<BlockInfo>, Option<BlockImportOutcome>)> + Send>>;

/// A future that resolves to a result containing a vector of [`ScrollPayloadAttributes`].
type DerivationPipelineFuture =
    Pin<Box<dyn Future<Output = Result<(), Box<dyn std::error::Error>>> + Send>>;

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
pub struct RollupNodeManager<C, EC, P, L1P> {
    /// The network manager that manages the scroll p2p network.
    network: NetworkManager,
    ///  The engine driver used to communicate with the engine.
    engine: Arc<EngineDriver<EC, P>>,
    /// The L1 provider used to retrieve L1 data.
    l1_provider: Arc<L1P>,
    /// The current derivation pipeline future.
    derivation_pipeline_future: Option<DerivationPipelineFuture>,
    /// The database which stores data from the [`Indexer`].
    database: Arc<Database>,
    /// A receiver for [`L1Notification`]s from the [`rollup_node_watcher::L1Watcher`].
    l1_notification_rx: Option<ReceiverStream<Arc<L1Notification>>>,
    /// An indexer used to index data for the rollup node.
    indexer: Indexer,
    /// The consensus algorithm used by the rollup node.
    consensus: C,
    /// The receiver for new blocks received from the network (used to bridge from eth-wire).
    new_block_rx: Option<UnboundedReceiverStream<NewBlockWithPeer>>,
    /// The forkchoice state of the rollup node.
    forkchoice_state: ForkchoiceState,
    /// A collection of pending block imports.
    pending_block_imports: FuturesOrdered<PendingBlockImportFuture>,
    /// An event sender for sending events to subscribers of the rollup node manager.
    event_sender: Option<EventSender<RollupEvent>>,
}

impl<C, EC, P, L1P> Debug for RollupNodeManager<C, EC, P, L1P>
where
    C: Debug,
    EC: Debug,
    P: Debug,
    L1P: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RollupNodeManager")
            .field("network", &self.network)
            .field("engine", &self.engine)
            .field("l1_provider", &self.l1_provider)
            .field(
                "derivation_pipeline_future",
                &self.derivation_pipeline_future.as_ref().map(|_| "Some({ ... })"),
            )
            .field("l1_notification_rx", &self.l1_notification_rx)
            .field("indexer", &self.indexer)
            .field("consensus", &self.consensus)
            .field("new_block_rx", &self.new_block_rx)
            .field("forkchoice_state", &self.forkchoice_state)
            .field("pending_block_imports", &self.pending_block_imports)
            .field("event_sender", &self.event_sender)
            .finish()
    }
}

impl<C, EC, P, L1P> RollupNodeManager<C, EC, P, L1P>
where
    C: Consensus + Unpin,
    EC: ScrollEngineApi<ScrollNetwork> + Unpin + Sync + Send + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
    L1P: L1Provider + Send + Sync + 'static,
{
    /// Create a new [`RollupNodeManager`] instance.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        network: NetworkManager,
        engine: EngineDriver<EC, P>,
        l1_provider: L1P,
        database: Arc<Database>,
        l1_notification_rx: Option<Receiver<Arc<L1Notification>>>,
        forkchoice_state: ForkchoiceState,
        consensus: C,
        new_block_rx: Option<UnboundedReceiver<NewBlockWithPeer>>,
    ) -> Self {
        let indexer = Indexer::new(database.clone());
        Self {
            network,
            engine: Arc::new(engine),
            l1_provider: Arc::new(l1_provider),
            derivation_pipeline_future: None,
            database,
            l1_notification_rx: l1_notification_rx.map(Into::into),
            indexer,
            consensus,
            new_block_rx: new_block_rx.map(Into::into),
            forkchoice_state,
            pending_block_imports: FuturesOrdered::new(),
            event_sender: None,
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
    pub fn handle_new_block(&mut self, block_with_peer: NewBlockWithPeer, cx: &mut Context<'_>) {
        if let Some(event_sender) = self.event_sender.as_ref() {
            event_sender.notify(RollupEvent::NewBlockReceived(block_with_peer.clone()));
        }

        let NewBlockWithPeer { peer_id: peer, block, signature } = block_with_peer;
        trace!(target: "scroll::node::manager", "Received new block from peer {:?} - hash {:?}", peer, block.hash_slow());

        // Validate the consensus of the block.
        // TODO: Should we spawn a task to validate the consensus of the block?
        //       Is the consensus validation blocking?
        if let Err(err) = self.consensus.validate_new_block(&block, &signature) {
            error!(target: "scroll::node::manager", ?err, "consensus checks failed on block {:?} from peer {:?}", block.hash_slow(), peer);
            self.network
                .handle()
                .block_import_outcome(BlockImportOutcome { peer, result: Err(err.into()) });
            return;
        }

        // If the forkchoice state is at genesis, update the forkchoice state with the parent of the
        // block.
        if self.forkchoice_state.is_genesis() {
            let block_num_hash = block.parent_num_hash();
            self.forkchoice_state = ForkchoiceState::from_block_info(BlockInfo {
                number: block_num_hash.number,
                hash: block_num_hash.hash,
            });
        }

        // Send the block to the engine to validate the correctness of the block.
        let mut fcs = self.get_alloy_fcs();
        let engine = self.engine.clone();
        let future = Box::pin(async move {
            trace!(target: "scroll::node::manager", "handling block import future for block {:?}", block.hash_slow());

            // convert the block to an execution payload and update the forkchoice state
            let execution_payload: ExecutionPayload =
                ExecutionPayloadV1::from_block_slow(&block).into();
            let unsafe_block_info: BlockInfo = (&execution_payload).into();
            fcs.head_block_hash = unsafe_block_info.hash;

            // process the execution payload
            // TODO: needs testing
            let (unsafe_block_info, import_outcome) =
                match engine.handle_execution_payload(execution_payload, fcs).await {
                    Err(EngineDriverError::InvalidExecutionPayload) => (
                        Some(unsafe_block_info),
                        Some(Err(BlockImportError::Validation(BlockValidationError::InvalidBlock))),
                    ),
                    Ok((PayloadStatusEnum::Valid, PayloadStatusEnum::Valid)) => (
                        Some(unsafe_block_info),
                        Some(Ok(BlockValidation::ValidBlock {
                            new_block: NewBlock {
                                block,
                                signature: signature.serialize_compact().to_vec().into(),
                            },
                        })),
                    ),
                    _ => (None, None),
                };

            (unsafe_block_info, import_outcome.map(|result| BlockImportOutcome { peer, result }))
        });

        self.pending_block_imports.push_back(future);
        cx.waker().wake_by_ref();
    }

    const fn get_alloy_fcs(&self) -> AlloyForkchoiceState {
        self.forkchoice_state.get_alloy_fcs()
    }

    /// Handles a network manager event.
    ///
    /// Currently the network manager only emits a `NewBlock` event.
    fn handle_network_manager_event(&mut self, event: NetworkManagerEvent, cx: &mut Context<'_>) {
        match event {
            NetworkManagerEvent::NewBlock(block) => self.handle_new_block(block, cx),
        }
    }

    /// Handles a block import outcome.
    fn handle_block_import_outcome(
        &mut self,
        unsafe_block_info: Option<BlockInfo>,
        outcome: Option<BlockImportOutcome>,
    ) {
        trace!(target: "scroll::node::manager", ?outcome, "handling block import outcome");

        // If we have an unsafe block info, update the forkchoice state.
        if let Some(unsafe_block_info) = unsafe_block_info {
            self.forkchoice_state.update_unsafe_block_info(unsafe_block_info);
        }

        // If we have an outcome, send it to the network manager.
        if let Some(outcome) = outcome {
            self.network.handle().block_import_outcome(outcome);
        }
    }

    /// Handles an [`L1Notification`] from the L1 watcher.
    fn handle_l1_notification(&mut self, notification: L1Notification) {
        self.indexer.handle_l1_notification(notification);
    }

    /// Handles the next batch from database.
    fn handle_next_batch(&mut self) {
        let fcs = self.get_alloy_fcs();
        let db = self.database.clone();
        let engine = self.engine.clone();
        let mut provider = self.l1_provider.clone();
        // TODO: how to fetch the current safe_block_info? We need a way to save the current block
        // infos (in the RNM?).
        // TODO: this should be removed.
        let mut safe_block_info = Default::default();

        let fut = Box::pin(async move {
            if let Some(batch) = db.get_next_unprocessed_batch().await? {
                trace!(target: "scroll::node::manager", batch_hash = ?batch.hash, index = ?batch.index, "processing batch");

                // derive the attributes from the batch.
                let hash = batch.hash;
                let attributes = derive(batch, &mut provider).await?;

                // handle all attributes by forwarding them to the engine driver.
                for attribute in attributes {
                    (safe_block_info, _) =
                        engine.handle_payload_attributes(safe_block_info, fcs, attribute).await?;
                }

                // mark the batch as processed in database.
                db.process_batch(hash).await?;
            }

            Ok(())
        });

        self.derivation_pipeline_future.replace(fut);
    }
}

impl<C, EC, P, L1P> Future for RollupNodeManager<C, EC, P, L1P>
where
    C: Consensus + Unpin,
    EC: ScrollEngineApi<ScrollNetwork> + Unpin + Sync + Send + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
    L1P: L1Provider + Unpin + Send + Sync + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Handle pending block imports.
        while let Poll::Ready(Some((block_info, outcome))) =
            this.pending_block_imports.poll_next_unpin(cx)
        {
            this.handle_block_import_outcome(block_info, outcome);
        }

        // Drain all L1 notifications.
        while let Some(Poll::Ready(Some(event))) =
            this.l1_notification_rx.as_mut().map(|x| x.poll_next_unpin(cx))
        {
            this.handle_l1_notification((*event).clone());
        }

        // Handle derivation pipeline.
        match this.derivation_pipeline_future.take() {
            Some(mut fut) => match fut.poll_unpin(cx) {
                Poll::Ready(res) => res.expect("failed to handle batch"),
                Poll::Pending => this.derivation_pipeline_future = Some(fut),
            },
            None => this.handle_next_batch(),
        }

        // Drain all Indexer events
        while let Poll::Ready(Some(event)) = this.indexer.poll_next_unpin(cx) {
            trace!(target: "scroll::node::manager", "Received indexer event: {:?}", event);
        }

        // Handle blocks received from the eth-wire protocol.
        while let Some(Poll::Ready(Some(block))) =
            this.new_block_rx.as_mut().map(|new_block_rx| new_block_rx.poll_next_unpin(cx))
        {
            this.handle_new_block(block, cx);
        }

        // Handle network manager events.
        while let Poll::Ready(Some(event)) = this.network.poll_next_unpin(cx) {
            this.handle_network_manager_event(event, cx);
        }

        Poll::Pending
    }
}
