//! This library contains the sequencer, which is responsible for sequencing transactions and
//! producing new blocks.

use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use alloy_eips::eip2718::Encodable2718;
use alloy_rpc_types_engine::{ExecutionData, PayloadAttributes, PayloadId};
use futures::{task::AtomicWaker, Stream};
use reth_scroll_engine_primitives::try_into_block;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::{BlockInfo, DEFAULT_BLOCK_DIFFICULTY};
use rollup_node_providers::{L1MessageProvider, L1ProviderError};
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_alloy_rpc_types_engine::{BlockDataHint, ScrollPayloadAttributes};
use scroll_engine::Engine;
use tokio::time::Interval;

mod config;
pub use config::{L1MessageInclusionMode, PayloadBuildingConfig, SequencerConfig};

mod error;
pub use error::SequencerError;

mod event;
pub use event::SequencerEvent;

mod metrics;
pub use metrics::SequencerMetrics;

/// A type alias for the payload building job future.
pub type PayloadBuildingJobFuture = Pin<Box<dyn Future<Output = PayloadId> + Send + Sync>>;

/// The sequencer is responsible for sequencing transactions and producing new blocks.
pub struct Sequencer<P, CS> {
    /// A reference to the provider.
    provider: Arc<P>,
    /// The configuration for the sequencer.
    config: SequencerConfig<CS>,
    /// The interval trigger for building a new block.
    trigger: Option<Interval>,
    /// The inflight payload building job
    payload_building_job: Option<PayloadBuildingJob>,
    /// The sequencer metrics.
    metrics: SequencerMetrics,
    /// A waker to notify when the Sequencer should be polled.
    waker: AtomicWaker,
}

impl<P, CS> Sequencer<P, CS>
where
    P: L1MessageProvider + Unpin + Send + Sync + 'static,
    CS: ScrollHardforks,
{
    /// Creates a new sequencer.
    pub fn new(provider: Arc<P>, config: SequencerConfig<CS>) -> Self {
        Self {
            provider,
            trigger: config.auto_start.then(|| delayed_interval(config.block_time)),
            config,
            payload_building_job: None,
            metrics: SequencerMetrics::default(),
            waker: AtomicWaker::new(),
        }
    }

    /// Returns a reference to the payload building job.
    pub const fn payload_building_job(&self) -> Option<&PayloadBuildingJob> {
        self.payload_building_job.as_ref()
    }

    /// Cancels the current payload building job, if any.
    pub fn cancel_payload_building_job(&mut self) {
        self.payload_building_job = None;
    }

    /// Enables the sequencer.
    pub fn enable(&mut self) {
        if self.trigger.is_none() {
            self.trigger = Some(delayed_interval(self.config.block_time));
        }
    }

    /// Disables the sequencer.
    pub fn disable(&mut self) {
        self.trigger = None;
        self.cancel_payload_building_job();
    }

    /// Creates a new block using the pending transactions from the message queue and
    /// the transaction pool.
    pub async fn start_payload_building<EC: ScrollEngineApi + Sync + Send + 'static>(
        &mut self,
        engine: &mut Engine<EC>,
    ) -> Result<(), SequencerError> {
        tracing::info!(target: "rollup_node::sequencer", "New payload attributes request received.");

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time can't go backwards")
            .as_secs();
        let payload_attributes = PayloadAttributes {
            timestamp,
            suggested_fee_recipient: self.config.fee_recipient,
            parent_beacon_block_root: None,
            prev_randao: Default::default(),
            withdrawals: None,
        };

        let now = Instant::now();
        let mut l1_messages = vec![];
        let mut cumulative_gas_used = 0;

        // Collect L1 messages to include in payload.
        let db_l1_messages = self
            .provider
            .get_n_messages(
                self.config.payload_building_config.l1_message_inclusion_mode.into(),
                self.config.payload_building_config.max_l1_messages_per_block,
            )
            .await
            .map_err(Into::<L1ProviderError>::into)?;

        let l1_origin = db_l1_messages.first().map(|msg| msg.l1_block_number);
        for msg in db_l1_messages {
            // TODO (greg): we only check the DA limit on the execution node side. We should also
            // check it here.
            let fits_in_block = msg.transaction.gas_limit + cumulative_gas_used <=
                self.config.payload_building_config.block_gas_limit;
            if !fits_in_block {
                break;
            }

            cumulative_gas_used += msg.transaction.gas_limit;
            l1_messages.push(msg.transaction.encoded_2718().into());
        }

        let payload_attributes = ScrollPayloadAttributes {
            payload_attributes,
            transactions: (!l1_messages.is_empty()).then_some(l1_messages),
            no_tx_pool: false,
            block_data_hint: BlockDataHint {
                difficulty: Some(DEFAULT_BLOCK_DIFFICULTY),
                ..Default::default()
            },
            // If setting the gas limit to None, the Reth payload builder will use the gas limit
            // passed via the `builder.gaslimit` CLI arg.
            gas_limit: None,
        };

        self.metrics.payload_attributes_building_duration.record(now.elapsed().as_secs_f64());

        // Request the engine to build a new payload.
        let fcu = engine.build_payload(None, payload_attributes).await?;
        let payload_id = fcu.payload_id.ok_or(SequencerError::MissingPayloadId)?;

        // Create a job that will wait for the configured duration before marking the payload as
        // ready.
        let payload_building_duration = self.config.payload_building_duration;
        self.payload_building_job = Some(PayloadBuildingJob {
            l1_origin,
            future: Box::pin(async move {
                // wait the configured duration for the execution node to build the payload.
                tokio::time::sleep(tokio::time::Duration::from_millis(payload_building_duration))
                    .await;
                payload_id
            }),
        });

        self.waker.wake();

        Ok(())
    }

    /// Handles a new payload by fetching it from the engine and updating the FCS head.
    pub async fn finalize_payload_building<EC: ScrollEngineApi + Sync + Send + 'static>(
        &mut self,
        payload_id: PayloadId,
        engine: &mut Engine<EC>,
    ) -> Result<Option<ScrollBlock>, SequencerError> {
        let payload = engine.get_payload(payload_id).await?;

        if payload.transactions.is_empty() && !self.config.allow_empty_blocks {
            tracing::trace!(target: "rollup_node::sequencer", "Built empty payload with id {payload_id:?}, discarding payload.");
            Ok(None)
        } else {
            tracing::info!(target: "rollup_node::sequencer", "Built payload with id {payload_id:?}, hash: {:#x}, number: {} containing {} transactions.", payload.block_hash, payload.block_number, payload.transactions.len());
            let block_info = BlockInfo { hash: payload.block_hash, number: payload.block_number };
            engine.update_fcs(Some(block_info), None, None).await?;
            let block: ScrollBlock = try_into_block(
                ExecutionData { payload: payload.into(), sidecar: Default::default() },
                self.config.chain_spec.clone(),
            )
            .map_err(|_| SequencerError::PayloadError)?;
            Ok(Some(block))
        }
    }
}

/// A job that builds a new payload.
pub struct PayloadBuildingJob {
    /// The L1 origin block number of the first included L1 message, if any.
    l1_origin: Option<u64>,
    /// The future that resolves to the payload ID once the job is complete.
    future: PayloadBuildingJobFuture,
}

impl std::fmt::Debug for PayloadBuildingJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayloadBuildingJob")
            .field("l1_origin", &self.l1_origin)
            .field("future", &"PayloadBuildingJobFuture")
            .finish()
    }
}

impl PayloadBuildingJob {
    /// Returns the L1 origin block number of the first included L1 message, if any.
    pub const fn l1_origin(&self) -> Option<u64> {
        self.l1_origin
    }
}

/// A stream that produces payload attributes.
impl<SMP, CS> Stream for Sequencer<SMP, CS> {
    type Item = SequencerEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.waker.register(cx.waker());

        // If there is an inflight payload building job, poll it.
        if let Some(payload_building_job) = this.payload_building_job.as_mut() {
            match payload_building_job.future.as_mut().poll(cx) {
                Poll::Ready(payload_id) => {
                    this.payload_building_job = None;
                    return Poll::Ready(Some(SequencerEvent::PayloadReady(payload_id)));
                }
                Poll::Pending => {}
            }
        }

        // Poll the trigger to see if it's time to build a new block.
        if let Some(trigger) = this.trigger.as_mut() {
            match trigger.poll_tick(cx) {
                Poll::Ready(_) => {
                    // If there's no inflight job, emit a new slot event.
                    if this.payload_building_job.is_none() {
                        return Poll::Ready(Some(SequencerEvent::NewSlot));
                    };
                    tracing::trace!(target: "rollup_node::sequencer", "Payload building job already in progress, skipping slot.");
                }
                Poll::Pending => {}
            }
        }

        Poll::Pending
    }
}

impl<SMP, CS: std::fmt::Debug> fmt::Debug for Sequencer<SMP, CS> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sequencer")
            .field("provider", &"SequencerMessageProvider")
            .field("config", &self.config)
            .field("payload_building_job", &"PayloadBuildingJob")
            .finish()
    }
}

/// Creates a delayed interval that will not skip ticks if the interval is missed but will delay
/// the next tick until the interval has passed.
fn delayed_interval(interval: u64) -> Interval {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(interval));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}
