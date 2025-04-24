use super::{future::EngineDriverFuture, ForkchoiceState};
use crate::{future::EngineDriverFutureResult, EngineDriverEvent};
use futures::{ready, task::AtomicWaker, Stream};
use rollup_node_primitives::{BlockInfo, ScrollPayloadAttributesWithBatchInfo};
use rollup_node_providers::ExecutionPayloadProvider;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_network::NewBlockWithPeer;
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::time::Duration;

/// The main interface to the Engine API of the EN.
/// Internally maintains the fork state of the chain.
pub struct EngineDriver<EC, P> {
    /// The engine API client.
    client: Arc<EC>,
    /// The execution payload provider
    execution_payload_provider: P,
    /// The fork choice state of the engine.
    fcs: ForkchoiceState,
    /// Block building duration.
    block_building_duration: Duration,
    /// The pending payload attributes derived from batches on L1.
    l1_payload_attributes: VecDeque<ScrollPayloadAttributesWithBatchInfo>,
    /// The pending block imports received over the network.
    block_imports: VecDeque<NewBlockWithPeer>,
    /// The payload attributes associated with the next block to be built.
    sequencer_payload_attributes: Option<ScrollPayloadAttributes>,
    /// The future of the engine driver.
    future: Option<EngineDriverFuture>,
    /// The waker to notify when the engine driver should be polled.
    waker: AtomicWaker,
}

impl<EC, P> EngineDriver<EC, P>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
{
    /// Create a new [`EngineDriver`] from the provided [`ScrollEngineApi`] and
    /// [`ExecutionPayloadProvider`].
    pub const fn new(
        client: Arc<EC>,
        execution_payload_provider: P,
        fcs: ForkchoiceState,
        block_building_duration: Duration,
    ) -> Self {
        Self {
            client,
            execution_payload_provider,
            fcs,
            block_building_duration,
            l1_payload_attributes: VecDeque::new(),
            block_imports: VecDeque::new(),
            sequencer_payload_attributes: None,
            future: None,
            waker: AtomicWaker::new(),
        }
    }

    /// Sets the finalized block info.
    pub fn set_finalized_block_info(&mut self, block_info: BlockInfo) {
        self.fcs.update_finalized_block_info(block_info);
    }

    /// Sets the payload building duration.
    pub fn set_payload_building_duration(&mut self, block_building_duration: Duration) {
        self.block_building_duration = block_building_duration;
    }

    /// Handles a block import request by adding it to the queue and waking up the driver.
    pub fn handle_block_import(&mut self, block_with_peer: NewBlockWithPeer) {
        tracing::trace!(target: "scroll::engine", ?block_with_peer, "new block import request received");
        self.block_imports.push_back(block_with_peer);
        self.waker.wake();
    }

    /// Handles a [`ScrollPayloadAttributes`] sourced from L1 by initiating a task sending the
    /// attribute to the EN via the [`EngineDriver`].
    pub fn handle_l1_consolidation(&mut self, attributes: ScrollPayloadAttributesWithBatchInfo) {
        self.l1_payload_attributes.push_back(attributes);
        self.waker.wake();
    }

    /// Handles a [`ScrollPayloadAttributes`] sourced from the sequencer by initiating a task
    /// sending the attributes to the EN and requesting a new payload to be built.
    pub fn handle_build_new_payload(&mut self, attributes: ScrollPayloadAttributes) {
        tracing::info!(target: "scroll::engine", ?attributes, "new payload attributes request received");

        if self.sequencer_payload_attributes.is_some() {
            tracing::error!(target: "scroll::engine", "a payload building job is already in progress");
            return;
        }

        self.sequencer_payload_attributes = Some(attributes);
        self.waker.wake();
    }

    /// This function is called when a future completes and is responsible for
    /// processing the result and returning an event if applicable.
    fn handle_future_result(
        &mut self,
        result: EngineDriverFutureResult,
    ) -> Option<EngineDriverEvent> {
        match result {
            EngineDriverFutureResult::BlockImport(result) => {
                tracing::info!(target: "scroll::engine", ?result, "handling block import result");

                match result {
                    Ok((block_info, block_import_outcome)) => {
                        // Update the unsafe block info
                        if let Some(block_info) = block_info {
                            tracing::trace!(target: "scroll::engine", ?block_info, "updating unsafe block info");
                            self.fcs.update_head_block_info(block_info);
                        };

                        // Return the block import outcome
                        return block_import_outcome.map(EngineDriverEvent::BlockImportOutcome)
                    }
                    Err(err) => {
                        tracing::error!(target: "scroll::engine", ?err, "failed to import block");
                    }
                }
            }
            EngineDriverFutureResult::L1Consolidation(result) => {
                tracing::info!(target: "scroll::engine", ?result, "handling L1 consolidation result");

                match result {
                    Ok((block_info, reorg, batch_info)) => {
                        // Update the safe block info and return the block info
                        tracing::trace!(target: "scroll::engine", ?block_info, "updating safe block info from block derived from L1");
                        self.fcs.update_safe_block_info(block_info);

                        // If we reorged, update the head block info
                        if reorg {
                            tracing::warn!(target: "scroll::engine", ?block_info, "reorging head to l1 derived block");
                            self.fcs.update_head_block_info(block_info);
                        }

                        return Some(EngineDriverEvent::L1BlockConsolidated((
                            block_info, batch_info,
                        )))
                    }
                    Err(err) => {
                        tracing::error!(target: "scroll::engine", ?err, "failed to consolidate block derived from L1")
                    }
                }
            }
            EngineDriverFutureResult::PayloadBuildingJob(result) => {
                tracing::info!(target: "scroll::engine", ?result, "handling payload building result");

                match result {
                    Ok(block) => {
                        // Update the unsafe block info and return the block
                        tracing::trace!(target: "scroll::engine", ?block, "updating unsafe block info from new payload");
                        let block_info = BlockInfo::new(block.number, block.hash_slow());
                        self.fcs.update_head_block_info(block_info);
                        return Some(EngineDriverEvent::NewPayload(block))
                    }
                    Err(err) => {
                        tracing::error!(target: "scroll::engine", ?err, "failed to build new payload")
                    }
                }
            }
        }

        None
    }
}

impl<EC, P> Stream for EngineDriver<EC, P>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Clone + Unpin + Send + Sync + 'static,
{
    type Item = EngineDriverEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Register the waker such that we can wake when required.
        this.waker.register(cx.waker());

        // If we have a future, poll it.
        if let Some(future) = this.future.as_mut() {
            let result = ready!(future.poll(cx));
            this.future = None;
            if let Some(event) = this.handle_future_result(result) {
                return Poll::Ready(Some(event));
            }
        };

        // If we have a payload building request from the sequencer, build a new payload.
        if let Some(payload_attributes) = this.sequencer_payload_attributes.take() {
            let fcs = this.fcs.get_alloy_fcs();
            let client = this.client.clone();
            let duration = this.block_building_duration;

            this.future = Some(EngineDriverFuture::payload_building_job(
                client,
                fcs,
                duration,
                payload_attributes,
            ));
            this.waker.wake();
            return Poll::Pending;
        }

        // Handle the block import requests.
        if let Some(block_with_peer) = this.block_imports.pop_front() {
            let fcs = this.fcs.get_alloy_fcs();
            let client = this.client.clone();

            this.future = Some(EngineDriverFuture::block_import(client, block_with_peer, fcs));
            this.waker.wake();
            return Poll::Pending;
        }

        if let Some(payload_attributes) = this.l1_payload_attributes.pop_front() {
            let safe_block_info = *this.fcs.safe_block_info();
            let fcs = this.fcs.get_alloy_fcs();
            let client = this.client.clone();
            let execution_payload_provider = this.execution_payload_provider.clone();

            this.future = Some(EngineDriverFuture::l1_consolidation(
                client,
                execution_payload_provider,
                safe_block_info,
                fcs,
                payload_attributes,
            ));
            this.waker.wake();
            return Poll::Pending;
        }

        Poll::Pending
    }
}

impl<EC, P> std::fmt::Debug for EngineDriver<EC, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineDriver")
            .field("client", &"ScrollEngineApi")
            .field("execution_payload_provider", &"ExecutionPayloadProvider")
            .field("fcs", &self.fcs)
            .field("future", &"EngineDriverFuture")
            .finish()
    }
}
