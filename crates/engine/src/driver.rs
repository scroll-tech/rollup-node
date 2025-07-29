use super::{future::EngineFuture, ForkchoiceState};
use crate::{
    future::{BuildNewPayloadFuture, EngineDriverFutureResult},
    metrics::EngineDriverMetrics,
    EngineDriverError, EngineDriverEvent,
};

use alloy_provider::Provider;
use futures::{ready, task::AtomicWaker, FutureExt, Stream};
use rollup_node_primitives::{
    BlockInfo, ChainImport, MeteredFuture, ScrollPayloadAttributesWithBatchInfo,
};
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_network::Scroll;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::time::Duration;

/// The main interface to the Engine API of the EN.
/// Internally maintains the fork state of the chain.
pub struct EngineDriver<EC, CS, P> {
    /// The engine API client.
    client: Arc<EC>,
    /// The chain spec.
    chain_spec: Arc<CS>,
    /// The provider.
    provider: Option<P>,
    /// The fork choice state of the engine.
    fcs: ForkchoiceState,
    /// Whether the EN is syncing.
    syncing: bool,
    /// Block building duration.
    block_building_duration: Duration,
    /// The pending payload attributes derived from batches on L1.
    l1_payload_attributes: VecDeque<ScrollPayloadAttributesWithBatchInfo>,
    /// The pending block imports received over the network.
    chain_imports: VecDeque<ChainImport>,
    /// The latest optimistic sync target.
    optimistic_sync_target: Option<BlockInfo>,
    /// The payload attributes associated with the next block to be built.
    sequencer_payload_attributes: Option<ScrollPayloadAttributes>,
    /// The future related to engine API.
    engine_future: Option<MeteredFuture<EngineFuture>>,
    /// The future for the payload building job.
    payload_building_future: Option<BuildNewPayloadFuture>,
    /// The driver metrics.
    metrics: EngineDriverMetrics,
    /// The waker to notify when the engine driver should be polled.
    waker: AtomicWaker,
}

impl<EC, CS, P> EngineDriver<EC, CS, P>
where
    EC: ScrollEngineApi + Sync + 'static,
    CS: ScrollHardforks + Sync + 'static,
    P: Provider<Scroll> + Clone + Sync + 'static,
{
    /// Create a new [`EngineDriver`].
    pub fn new(
        client: Arc<EC>,
        chain_spec: Arc<CS>,
        provider: Option<P>,
        fcs: ForkchoiceState,
        sync_at_start_up: bool,
        block_building_duration: Duration,
    ) -> Self {
        Self {
            client,
            chain_spec,
            provider,
            fcs,
            block_building_duration,
            syncing: sync_at_start_up,
            l1_payload_attributes: VecDeque::new(),
            chain_imports: VecDeque::new(),
            optimistic_sync_target: None,
            sequencer_payload_attributes: None,
            payload_building_future: None,
            engine_future: None,
            metrics: EngineDriverMetrics::default(),
            waker: AtomicWaker::new(),
        }
    }

    /// Sets the finalized block info.
    pub fn set_finalized_block_info(&mut self, block_info: BlockInfo) {
        self.fcs.update_finalized_block_info(block_info);
    }

    /// Sets the safe block info.
    pub fn set_safe_block_info(&mut self, block_info: BlockInfo) {
        self.fcs.update_safe_block_info(block_info);
    }

    /// Sets the head block info.
    pub fn set_head_block_info(&mut self, block_info: BlockInfo) {
        self.fcs.update_head_block_info(block_info);
    }

    /// Sets the payload building duration.
    pub fn set_payload_building_duration(&mut self, block_building_duration: Duration) {
        self.block_building_duration = block_building_duration;
    }

    /// Clear the l1 attributes queue.
    pub fn clear_l1_payload_attributes(&mut self) {
        // clear the L1 attributes queue.
        self.l1_payload_attributes.clear();

        // drop the engine future if it is a L1 consolidation.
        if let Some(MeteredFuture { fut: EngineFuture::L1Consolidation(_), .. }) =
            self.engine_future
        {
            self.engine_future.take();
        }
    }

    /// Handles a block import request by adding it to the queue and waking up the driver.
    pub fn handle_chain_import(&mut self, chain_import: ChainImport) {
        tracing::trace!(target: "scroll::engine", head = %chain_import.chain.last().unwrap().hash_slow(), "new block import request received");

        self.chain_imports.push_back(chain_import);
        self.waker.wake();
    }

    /// Optimistically syncs the chain to the provided block info.
    pub fn handle_optimistic_sync(&mut self, block_info: BlockInfo) {
        tracing::info!(target: "scroll::engine", ?block_info, "optimistic sync request received");

        // Purge all pending block imports.
        self.chain_imports.clear();

        // Update the fork choice state with the new block info.
        self.optimistic_sync_target = Some(block_info);

        // Wake up the driver to process the optimistic sync.
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
    fn handle_engine_future_result(
        &mut self,
        result: EngineDriverFutureResult,
        duration: Duration,
    ) -> Option<EngineDriverEvent> {
        match result {
            EngineDriverFutureResult::BlockImport(result) => {
                tracing::info!(target: "scroll::engine", ?result, "handling block import result");

                match result {
                    Ok((block_info, block_import_outcome, payload_status)) => {
                        // Update the unsafe block info
                        if let Some(block_info) = block_info {
                            tracing::trace!(target: "scroll::engine", ?block_info, "updating unsafe block info");
                            self.fcs.update_head_block_info(block_info);
                        };

                        // Update the sync status
                        if !payload_status.is_syncing() {
                            tracing::trace!(target: "scroll::engine", "sync finished");
                            self.syncing = false;
                        }

                        // record the metric.
                        self.metrics.block_import_duration.record(duration.as_secs_f64());

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
                    Ok(consolidation_outcome) => {
                        let block_info = consolidation_outcome.block_info();

                        // Update the safe block info and return the block info
                        tracing::trace!(target: "scroll::engine", ?block_info, "updating safe block info from block derived from L1");
                        self.fcs.update_safe_block_info(block_info.block_info);

                        // If we reorged, update the head block info
                        if consolidation_outcome.is_reorg() {
                            tracing::warn!(target: "scroll::engine", ?block_info, "reorging head to l1 derived block");
                            self.fcs.update_head_block_info(block_info.block_info);
                        }

                        // record the metric.
                        self.metrics.l1_consolidation_duration.record(duration.as_secs_f64());

                        return Some(EngineDriverEvent::L1BlockConsolidated(consolidation_outcome))
                    }
                    Err(err) => {
                        tracing::error!(target: "scroll::engine", ?err, "failed to consolidate block derived from L1");
                        if let EngineDriverError::MissingPayloadId(attributes) = err {
                            self.l1_payload_attributes.push_front(attributes);
                        }
                    }
                }
            }
            EngineDriverFutureResult::PayloadBuildingJob(result) => {
                tracing::info!(target: "scroll::engine", result = ?result.as_ref().map(|b| b.header.as_ref()), "handling payload building result");

                match result {
                    Ok(block) => {
                        // Update the unsafe block info and return the block
                        let block_info = BlockInfo::new(block.number, block.hash_slow());
                        tracing::trace!(target: "scroll::engine", ?block_info, "updating unsafe block info from new payload");
                        self.fcs.update_head_block_info(block_info);

                        // record the metrics.
                        self.metrics.build_new_payload_duration.record(duration.as_secs_f64());
                        self.metrics.gas_per_block.record(block.gas_used as f64);

                        return Some(EngineDriverEvent::NewPayload(block))
                    }
                    Err(err) => {
                        tracing::error!(target: "scroll::engine", ?err, "failed to build new payload");
                        if let EngineDriverError::MissingPayloadId(attributes) = err {
                            self.l1_payload_attributes.push_front(attributes);
                        }
                    }
                }
            }
            EngineDriverFutureResult::OptimisticSync(result) => {
                tracing::info!(target: "scroll::engine", ?result, "handling optimistic sync result");

                match result {
                    Err(err) => {
                        tracing::error!(target: "scroll::engine", ?err, "failed to perform optimistic sync")
                    }
                    Ok(fcu) => {
                        tracing::trace!(target: "scroll::engine", ?fcu, "optimistic sync issued successfully");
                    }
                }
            }
        }

        None
    }

    /// A helper function to check if a payload building job is in progress.
    pub const fn is_payload_building_in_progress(&self) -> bool {
        self.sequencer_payload_attributes.is_some() || self.payload_building_future.is_some()
    }

    /// Returns the sync status.
    pub const fn is_syncing(&self) -> bool {
        self.syncing
    }

    /// Returns the forkchoice state.
    pub const fn forkchoice_state(&self) -> &ForkchoiceState {
        &self.fcs
    }

    /// Returns the alloy forkchoice state.
    pub fn alloy_forkchoice_state(&self) -> alloy_rpc_types_engine::ForkchoiceState {
        if self.is_syncing() {
            self.fcs.get_alloy_optimistic_fcs()
        } else {
            self.fcs.get_alloy_fcs()
        }
    }
}

impl<EC, CS, P> Stream for EngineDriver<EC, CS, P>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    CS: ScrollHardforks + Send + Sync + 'static,
    P: Provider<Scroll> + Clone + Unpin + Send + Sync + 'static,
{
    type Item = EngineDriverEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Register the waker such that we can wake when required.
        this.waker.register(cx.waker());

        // If we have a future, poll it.
        if let Some(future) = this.engine_future.as_mut() {
            let (duration, result) = ready!(future.poll_unpin(cx));
            this.engine_future = None;
            if let Some(event) = this.handle_engine_future_result(result, duration) {
                return Poll::Ready(Some(event));
            }
        };

        // Take the handle to the payload building job if it exists and poll it.
        if let Some(mut handle) = this.payload_building_future.take() {
            // If the payload build job is done, handle the result - otherwise continue to process
            // another driver job.
            match handle.poll_unpin(cx) {
                Poll::Ready((duration, result)) => match result {
                    Ok(block) => {
                        this.engine_future = Some(
                            MeteredFuture::new(EngineFuture::handle_new_payload_job(
                                this.client.clone(),
                                this.alloy_forkchoice_state(),
                                block,
                            ))
                            .with_initial_duration(duration),
                        );
                        this.waker.wake();
                    }
                    Err(err) => {
                        tracing::error!(target: "scroll::engine", ?err, "failed to build new payload");
                    }
                },
                // The job is still in progress, reassign the handle and continue.
                _ => {
                    this.payload_building_future = Some(handle);
                }
            }
        }

        // If we have a payload building request from the sequencer, build a new payload.
        if let Some(payload_attributes) = this.sequencer_payload_attributes.take() {
            let fcs = this.alloy_forkchoice_state();
            let client = this.client.clone();
            let chain_spec = this.chain_spec.clone();
            let duration = this.block_building_duration;

            this.payload_building_future =
                Some(MeteredFuture::new(Box::pin(super::future::build_new_payload(
                    client,
                    chain_spec,
                    fcs,
                    duration,
                    payload_attributes,
                ))));
            this.waker.wake();
            return Poll::Pending;
        }

        // If we have an optimistic sync target, issue the optimistic sync.
        if let Some(block_info) = this.optimistic_sync_target.take() {
            this.fcs.update_head_block_info(block_info);
            let fcs = this.fcs.get_alloy_optimistic_fcs();
            this.engine_future =
                Some(MeteredFuture::new(EngineFuture::optimistic_sync(this.client.clone(), fcs)));
            this.waker.wake();
            return Poll::Pending;
        }

        // Handle the chain import requests.
        if let Some(chain_import) = this.chain_imports.pop_front() {
            let fcs = this.alloy_forkchoice_state();
            let client = this.client.clone();

            this.engine_future =
                Some(MeteredFuture::new(EngineFuture::chain_import(client, chain_import, fcs)));

            this.waker.wake();
            return Poll::Pending;
        }

        if let Some(payload_attributes) = this.l1_payload_attributes.pop_front() {
            let fcs = this.fcs.clone();
            let client = this.client.clone();

            if let Some(provider) = this.provider.clone() {
                this.engine_future = Some(MeteredFuture::new(EngineFuture::l1_consolidation(
                    client,
                    provider,
                    fcs,
                    payload_attributes,
                )));
                this.waker.wake();
            } else {
                tracing::error!(target: "scroll::engine", "l1 consolidation requires an execution payload provider");
            }

            return Poll::Pending;
        }

        Poll::Pending
    }
}

impl<EC, CS, P> std::fmt::Debug for EngineDriver<EC, CS, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineDriver")
            .field("client", &"ScrollEngineApi")
            .field("provider", &"ExecutionPayloadProvider")
            .field("chain_spec", &"ScrollHardforks")
            .field("fcs", &self.fcs)
            .field("future", &"EngineDriverFuture")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::future::build_new_payload;

    use reth_scroll_chainspec::SCROLL_DEV;
    use rollup_node_providers::ScrollRootProvider;
    use scroll_engine::test_utils::PanicEngineClient;

    impl<EC, P, CS> EngineDriver<EC, P, CS> {
        fn with_payload_future(&mut self, future: BuildNewPayloadFuture) {
            self.payload_building_future = Some(future);
        }
    }

    #[tokio::test]
    async fn test_is_payload_building_in_progress() {
        let client = Arc::new(PanicEngineClient);
        let chain_spec = SCROLL_DEV.clone();
        let fcs =
            ForkchoiceState::from_block_info(BlockInfo { number: 0, hash: Default::default() });
        let duration = Duration::from_secs(2);

        let mut driver =
            EngineDriver::new(client, chain_spec, None::<ScrollRootProvider>, fcs, false, duration);

        // Initially, it should be false
        assert!(!driver.is_payload_building_in_progress());

        // Simulate a payload building job invocation
        driver.handle_build_new_payload(Default::default());

        // Now, it should return true
        assert!(driver.is_payload_building_in_progress());
    }

    #[tokio::test]
    async fn test_is_payload_building_in_progress_with_future() {
        let client = Arc::new(PanicEngineClient);
        let chain_spec = SCROLL_DEV.clone();
        let fcs =
            ForkchoiceState::from_block_info(BlockInfo { number: 0, hash: Default::default() });
        let duration = Duration::from_secs(2);

        let mut driver = EngineDriver::new(
            client.clone(),
            chain_spec.clone(),
            None::<ScrollRootProvider>,
            fcs,
            false,
            duration,
        );

        // Initially, it should be false
        assert!(!driver.is_payload_building_in_progress());

        // Set a future to simulate an ongoing job
        driver.with_payload_future(MeteredFuture::new(Box::pin(build_new_payload(
            client,
            chain_spec,
            Default::default(),
            Default::default(),
            Default::default(),
        ))));

        // Now, it should return true
        assert!(driver.is_payload_building_in_progress());
    }
}
