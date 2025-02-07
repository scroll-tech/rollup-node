//! This library contains the main manager for the rollup node.

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use alloy_rpc_types_engine::{
    ExecutionPayload, ExecutionPayloadV1, ForkchoiceState as AlloyForkchoiceState,
};
use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use scroll_alloy_network::Scroll as ScrollNetwork;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_engine::{
    BlockInfo, EngineDriver, EngineDriverError, ExecutionPayloadProvider, ForkchoiceState,
};
use scroll_network::{
    BlockImportError, BlockImportOutcome, BlockValidation, BlockValidationError, NetworkManager,
    NetworkManagerEvent, NewBlockWithPeer,
};
use scroll_wire::NewBlock;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, trace};

mod consensus;
use consensus::Consensus;
pub use consensus::PoAConsensus;

// mod error;
// use error::EngineManagerError;

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
/// - `pending_block_imports`: A list of pending block imports.
#[derive(Debug)]
pub struct RollupNodeManager<C, EC, P> {
    /// The network handle used to communicate with the network manager.
    network: NetworkManager,
    engine: Arc<EngineDriver<EC, P>>,
    consensus: C,
    new_block_rx: UnboundedReceiverStream<NewBlockWithPeer>,
    forkchoice_state: ForkchoiceState,
    pending_block_imports:
        FuturesUnordered<Pin<Box<dyn Future<Output = BlockImportOutcome> + Send>>>,
}

impl<C, EC, P> RollupNodeManager<C, EC, P>
where
    C: Consensus + Unpin,
    EC: ScrollEngineApi<ScrollNetwork> + Unpin + Sync + Send + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
{
    /// Create a new [`RollupNodeManager`] instance.
    pub fn new(
        network: NetworkManager,
        engine: EngineDriver<EC, P>,
        forkchoice_state: ForkchoiceState,
        consensus: C,
        new_block_rx: UnboundedReceiver<NewBlockWithPeer>,
    ) -> Self {
        Self {
            network,
            engine: Arc::new(engine),
            consensus,
            new_block_rx: new_block_rx.into(),
            forkchoice_state,
            pending_block_imports: FuturesUnordered::new(),
        }
    }

    /// Handles a new block received from the network.
    ///
    /// We will first validate the consensus of the block, then we will send the block to the engine
    /// to validate the correctness of the block.
    pub fn handle_new_block(&mut self, block_with_peer: NewBlockWithPeer) {
        let NewBlockWithPeer { peer_id: peer, block, signature } = block_with_peer;

        trace!("Received new block from peer {:?}", peer);

        // Validate the consensus of the block.
        // TODO: Should we spawn a task to validate the consensus of the block?
        //       Is the consensus validation blocking?
        if let Err(err) = self.consensus.validate_new_block(&block, &signature) {
            error!(target: "manager::RollupNodeManager", ?err, "consensus checks failed on block {:?} from peer {:?}", block.hash_slow(), peer);
            self.network
                .handle()
                .block_import_outcome(BlockImportOutcome { peer, result: Err(err) });
            return;
        }

        // Update the forkchoice state with the new block.
        let execution_payload: ExecutionPayload =
            ExecutionPayloadV1::from_block_slow(&block).into();
        let unsafe_block_info: BlockInfo = (&execution_payload).into();
        self.forkchoice_state.update_unsafe_block_info(unsafe_block_info);
        let fcs = self.get_fcs();

        // Send the block to the engine to validate the correctness of the block.
        let engine = self.engine.clone();
        let future = Box::pin(async move {
            trace!(target: "scroll_rollup_manager::RollupNodeManager", "handling block import future");
            let result = match engine.handle_execution_payload(execution_payload, fcs).await {
                Ok(_) => Ok(BlockValidation::ValidBlock {
                    new_block: NewBlock {
                        block,
                        signature: signature.serialize_compact().to_vec().into(),
                    },
                }),
                // TODO: Implement From<EngineDriverError> for BlockImportError
                Err(EngineDriverError::InvalidExecutionPayload) => {
                    Err(BlockImportError::Validation(BlockValidationError::InvalidBlock))
                }
                Err(EngineDriverError::EngineUnavailable | EngineDriverError::FcuFailed) => {
                    Err(BlockImportError::Validation(BlockValidationError::EngineApiError))
                }
            };
            BlockImportOutcome { peer, result }
        });

        self.pending_block_imports.push(future);
    }

    const fn get_fcs(&self) -> AlloyForkchoiceState {
        AlloyForkchoiceState {
            head_block_hash: self.forkchoice_state.unsafe_block_info().hash,
            safe_block_hash: self.forkchoice_state.safe_block_info().hash,
            finalized_block_hash: self.forkchoice_state.finalized_block_info().hash,
        }
    }

    /// Handles a network manager event.
    fn handle_network_manager_event(&mut self, event: NetworkManagerEvent) {
        match event {
            NetworkManagerEvent::NewBlock(block) => self.handle_new_block(block),
        }
    }

    fn handle_block_import_outcome(&self, outcome: BlockImportOutcome) {
        trace!(target: "scroll_rollup_manager::RollupNodeManager", ?outcome, "handling block import outcome");
        self.network.handle().block_import_outcome(outcome);
    }
}

impl<C, EC, P> Future for RollupNodeManager<C, EC, P>
where
    C: Consensus + Unpin,
    EC: ScrollEngineApi<ScrollNetwork> + Unpin + Sync + Send + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        trace!("Polling RollupNodeManager");

        // Handle pending block imports.
        while let Poll::Ready(Some(outcome)) = this.pending_block_imports.poll_next_unpin(cx) {
            this.handle_block_import_outcome(outcome);
        }

        // Handle blocks received from the eth-wire protocol.
        while let Poll::Ready(Some(block)) = this.new_block_rx.poll_next_unpin(cx) {
            this.handle_new_block(block);
            cx.waker().wake_by_ref();
        }

        // Handle network manager events.
        while let Poll::Ready(event) = this.network.poll_unpin(cx) {
            this.handle_network_manager_event(event);
        }

        Poll::Pending
    }
}
