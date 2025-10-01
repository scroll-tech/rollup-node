//! This library contains the sequencer, which is responsible for sequencing transactions and
//! producing new blocks.

use std::{
    fmt,
    future::Future,
    pin::Pin,
    str::FromStr,
    sync::Arc,
    task::{Context, Poll},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::Address;
use alloy_rpc_types_engine::PayloadAttributes;
use futures::{task::AtomicWaker, Stream};
use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::{L1MessageEnvelope, DEFAULT_BLOCK_DIFFICULTY};
use rollup_node_providers::{L1MessageProvider, L1ProviderError};
use scroll_alloy_consensus::ScrollTransaction;
use scroll_alloy_rpc_types_engine::{BlockDataHint, ScrollPayloadAttributes};

mod error;
pub use error::SequencerError;

mod metrics;
pub use metrics::SequencerMetrics;
use scroll_db::L1MessageKey;

/// A type alias for the payload building job future.
pub type PayloadBuildingJobFuture =
    Pin<Box<dyn Future<Output = Result<ScrollPayloadAttributes, SequencerError>> + Send>>;

/// Configuration for L1 message inclusion strategy.
#[derive(Debug, Default, Clone, Copy)]
pub enum L1MessageInclusionMode {
    /// Include L1 messages based on block depth.
    BlockDepth(u64),
    /// Include only finalized L1 messages.
    #[default]
    Finalized,
}

impl FromStr for L1MessageInclusionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("finalized") {
            Ok(Self::Finalized)
        } else if let Some(rest) = s.strip_prefix("depth:") {
            rest.parse::<u64>()
                .map(Self::BlockDepth)
                .map_err(|_| format!("Expected a valid number after 'depth:', got '{rest}'"))
        } else {
            Err("Expected 'finalized' or 'depth:{number}' (e.g. 'depth:10')".to_string())
        }
    }
}

impl fmt::Display for L1MessageInclusionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finalized => write!(f, "finalized"),
            Self::BlockDepth(depth) => write!(f, "depth:{depth}"),
        }
    }
}

/// The sequencer is responsible for sequencing transactions and producing new blocks.
pub struct Sequencer<P> {
    /// A reference to the database.
    provider: Arc<P>,
    /// The fee recipient.
    fee_recipient: Address,
    /// The block gas limit.
    block_gas_limit: u64,
    /// The number of L1 messages to include in each block.
    max_l1_messages_per_block: u64,
    /// The current l1 block number.
    l1_block_number: u64,
    /// The L1 finalized block number.
    l1_finalized_block_number: u64,
    /// The L1 message inclusion mode configuration.
    l1_message_inclusion_mode: L1MessageInclusionMode,
    /// The inflight payload attributes job
    payload_attributes_job: Option<PayloadBuildingJobFuture>,
    /// The current L1 messages queue index.
    l1_messages_queue_index: u64,
    /// The sequencer metrics.
    metrics: SequencerMetrics,
    /// A waker to notify when the Sequencer should be polled.
    waker: AtomicWaker,
}

impl<P> Sequencer<P>
where
    P: L1MessageProvider + Unpin + Send + Sync + 'static,
{
    /// Creates a new sequencer.
    pub fn new(
        provider: Arc<P>,
        fee_recipient: Address,
        block_gas_limit: u64,
        max_l1_messages_per_block: u64,
        l1_block_number: u64,
        l1_message_inclusion_mode: L1MessageInclusionMode,
        l1_messages_queue_index: u64,
    ) -> Self {
        Self {
            provider,
            fee_recipient,
            block_gas_limit,
            max_l1_messages_per_block,
            l1_block_number,
            l1_finalized_block_number: 0,
            l1_message_inclusion_mode,
            l1_messages_queue_index,
            payload_attributes_job: None,
            metrics: SequencerMetrics::default(),
            waker: AtomicWaker::new(),
        }
    }

    /// Set the L1 finalized block number.
    pub fn set_l1_finalized_block_number(&mut self, l1_finalized_block_number: u64) {
        self.l1_finalized_block_number = l1_finalized_block_number;
    }

    /// Creates a new block using the pending transactions from the message queue and
    /// the transaction pool.
    pub fn build_payload_attributes(&mut self) {
        tracing::info!(target: "rollup_node::sequencer", "New payload attributes request received.");

        if self.payload_attributes_job.is_some() {
            tracing::error!(target: "rollup_node::sequencer", "A payload attributes building job is already in progress");
            return;
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time can't go backwards")
            .as_secs();
        let payload_attributes = PayloadAttributes {
            timestamp,
            suggested_fee_recipient: self.fee_recipient,
            parent_beacon_block_root: None,
            prev_randao: Default::default(),
            withdrawals: None,
        };
        let max_l1_messages = self.max_l1_messages_per_block;
        let database = self.provider.clone();
        let block_gas_limit = self.block_gas_limit;
        let l1_block_number = self.l1_block_number;
        let l1_finalized_block_number = self.l1_finalized_block_number;
        let l1_message_inclusion_mode = self.l1_message_inclusion_mode;
        let l1_messages_queue_index = self.l1_messages_queue_index;
        let metrics = self.metrics.clone();

        self.payload_attributes_job = Some(Box::pin(async move {
            let now = Instant::now();
            let res = build_payload_attributes(
                database,
                max_l1_messages,
                payload_attributes,
                block_gas_limit,
                l1_block_number,
                l1_finalized_block_number,
                l1_message_inclusion_mode,
                l1_messages_queue_index,
            )
            .await;
            metrics.payload_attributes_building_duration.record(now.elapsed().as_secs_f64());
            res
        }));

        self.waker.wake();
    }

    /// Handle a reorg event.
    pub fn handle_reorg(&mut self, queue_index: Option<u64>, l1_block_number: u64) {
        if let Some(index) = queue_index {
            self.l1_messages_queue_index = index;
        }
        self.l1_block_number = l1_block_number;
    }

    /// Handle a new L1 block.
    pub fn handle_new_l1_block(&mut self, block_number: u64) {
        self.l1_block_number = block_number;
    }

    /// Handle new payload by updating the L1 messages queue index.
    pub fn handle_new_payload(&mut self, block: &ScrollBlock) {
        let queue_index = block.body.transactions.iter().filter_map(|tx| tx.queue_index()).max();
        if let Some(queue_index) = queue_index {
            // only update the queue index if it has advanced
            if queue_index + 1 > self.l1_messages_queue_index {
                tracing::trace!(target: "rollup_node::sequencer", "Advancing L1 messages queue index from {} to {}", self.l1_messages_queue_index, queue_index + 1);
                self.l1_messages_queue_index = queue_index + 1;
            } else {
                tracing::warn!(target: "rollup_node::sequencer", "Skipping L1 messages queue index update, current index is {}, new payload has max index {}", self.l1_messages_queue_index, queue_index);
            }
        }
    }
}

/// A stream that produces payload attributes.
impl<SMP> Stream for Sequencer<SMP> {
    type Item = ScrollPayloadAttributes;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());

        if let Some(payload_building_job) = self.payload_attributes_job.as_mut() {
            match payload_building_job.as_mut().poll(cx) {
                Poll::Ready(Ok(block)) => {
                    self.payload_attributes_job = None;
                    Poll::Ready(Some(block))
                }
                Poll::Ready(Err(err)) => {
                    tracing::error!(target: "rollup_node::sequencer", "Error building payload attributes: {err}");
                    self.payload_attributes_job = None;
                    Poll::Ready(None)
                }
                Poll::Pending => Poll::Pending,
            }
        } else {
            Poll::Pending
        }
    }
}

/// Builds the payload attributes for the sequencer using the given L1 message provider.
/// It collects the L1 messages to include in the payload and returns a `ScrollPayloadAttributes`
/// instance.
#[allow(clippy::too_many_arguments)]
async fn build_payload_attributes<P: L1MessageProvider + Unpin + Send + Sync + 'static>(
    provider: Arc<P>,
    max_l1_messages: u64,
    payload_attributes: PayloadAttributes,
    block_gas_limit: u64,
    current_l1_block_number: u64,
    l1_finalized_block_number: u64,
    l1_message_inclusion_mode: L1MessageInclusionMode,
    l1_messages_queue_index: u64,
) -> Result<ScrollPayloadAttributes, SequencerError> {
    let mut l1_messages = vec![];
    let mut cumulative_gas_used = 0;
    let mut expected_index = l1_messages_queue_index;

    // Collect L1 messages to include in payload.
    let db_l1_messages = provider
        .get_n_messages(L1MessageKey::from_queue_index(l1_messages_queue_index), max_l1_messages)
        .await
        .map_err(Into::<L1ProviderError>::into)?;

    for msg in db_l1_messages {
        // TODO (greg): we only check the DA limit on the execution node side. We should also check
        // it here.
        let fits_in_block = msg.transaction.gas_limit + cumulative_gas_used <= block_gas_limit;
        let l1_inclusion_requirement_met = meets_l1_inclusion_requirement(
            &msg,
            l1_message_inclusion_mode,
            current_l1_block_number,
            l1_finalized_block_number,
        );
        if !fits_in_block || !l1_inclusion_requirement_met {
            break;
        }

        // Defensively ensure L1 messages are contiguous.
        if msg.transaction.queue_index != expected_index {
            return Err(SequencerError::NonContiguousL1Messages {
                expected: expected_index,
                got: msg.transaction.queue_index,
            });
        }

expected_index += 1;
        cumulative_gas_used += msg.transaction.gas_limit;
        l1_messages.push(msg.transaction.encoded_2718().into());
    }

    Ok(ScrollPayloadAttributes {
        payload_attributes,
        transactions: (!l1_messages.is_empty()).then_some(l1_messages),
        no_tx_pool: false,
        block_data_hint: BlockDataHint {
            difficulty: Some(DEFAULT_BLOCK_DIFFICULTY),
            ..Default::default()
        },
        // If setting the gas limit to None, the Reth payload builder will use the gas limit passed
        // via the `builder.gaslimit` CLI arg.
        gas_limit: None,
    })
}

/// Returns true if the L1 message should be included in the payload based on the inclusion mode,
/// the current L1 block number and the L1 finalized block number.
const fn meets_l1_inclusion_requirement(
    l1_msg: &L1MessageEnvelope,
    inclusion_mode: L1MessageInclusionMode,
    current_l1_block_number: u64,
    l1_finalized_block_number: u64,
) -> bool {
    match inclusion_mode {
        L1MessageInclusionMode::BlockDepth(depth) => {
            l1_msg.l1_block_number + depth <= current_l1_block_number
        }
        L1MessageInclusionMode::Finalized => l1_msg.l1_block_number <= l1_finalized_block_number,
    }
}

impl<SMP> fmt::Debug for Sequencer<SMP> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sequencer")
            .field("provider", &"SequencerMessageProvider")
            .field("fee_recipient", &self.fee_recipient)
            .field("payload_building_job", &"PayloadBuildingJob")
            .field("l1_message_per_block", &self.max_l1_messages_per_block)
            .field("l1_message_inclusion_mode", &self.l1_message_inclusion_mode)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scroll_alloy_consensus::TxL1Message;

    #[test]
    fn test_l1_message_predicate() {
        // block depth not met.
        assert!(!meets_l1_inclusion_requirement(
            &L1MessageEnvelope {
                transaction: Default::default(),
                l1_block_number: 10,
                l2_block_number: None,
                queue_hash: None,
            },
            L1MessageInclusionMode::BlockDepth(5),
            10,
            10,
        ));

        // block depth met.
        assert!(meets_l1_inclusion_requirement(
            &L1MessageEnvelope {
                transaction: TxL1Message { gas_limit: 1, ..Default::default() },
                l1_block_number: 5,
                l2_block_number: None,
                queue_hash: None,
            },
            L1MessageInclusionMode::BlockDepth(5),
            10,
            10,
        ));

        // not finalized.
        assert!(!meets_l1_inclusion_requirement(
            &L1MessageEnvelope {
                transaction: TxL1Message { gas_limit: 1, ..Default::default() },
                l1_block_number: 15,
                l2_block_number: None,
                queue_hash: None,
            },
            L1MessageInclusionMode::Finalized,
            10,
            10,
        ));

        // finalized.
        assert!(meets_l1_inclusion_requirement(
            &L1MessageEnvelope {
                transaction: TxL1Message { gas_limit: 1, ..Default::default() },
                l1_block_number: 10,
                l2_block_number: None,
                queue_hash: None,
            },
            L1MessageInclusionMode::Finalized,
            10,
            10,
        ));
    }
}
