//! This library contains the sequencer, which is responsible for sequencing transactions and
//! producing new blocks.

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::Address;
use alloy_rpc_types_engine::PayloadAttributes;
use futures::Stream;
use rollup_node_providers::{L1MessageDelayProvider, L1MessageProvider};
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use std::task::{Context, Poll, Waker};

mod error;
pub use error::SequencerError;

/// A type alias for the payload building job future.
pub type PayloadBuildingJobFuture =
    Pin<Box<dyn Future<Output = Result<ScrollPayloadAttributes, SequencerError>> + Send>>;

/// A trait used to define the L1 message provider for the sequencer.
pub trait SequencerL1MessageProvider: L1MessageProvider + L1MessageDelayProvider {}
impl<T> SequencerL1MessageProvider for T where T: L1MessageProvider + L1MessageDelayProvider {}

/// The sequencer is responsible for sequencing transactions and producing new blocks.
pub struct Sequencer<SMP> {
    /// A reference to the database
    provider: Arc<SMP>,
    /// The fee recipient
    fee_recipient: Address,
    /// The number of L1 messages to include in each block.
    max_l1_messages_per_block: u64,
    /// The inflight payload attributes job
    payload_attributes_job: Option<PayloadBuildingJobFuture>,
    /// A waker to notify when the Sequencer should be polled.
    waker: Option<Waker>,
}

impl<SMP> Sequencer<SMP>
where
    SMP: SequencerL1MessageProvider + Unpin + Send + Sync + 'static,
{
    /// Creates a new sequencer.
    pub fn new(provider: Arc<SMP>, fee_recipient: Address, max_l1_messages_per_block: u64) -> Self {
        Self {
            provider,
            fee_recipient,
            max_l1_messages_per_block,
            payload_attributes_job: None,
            waker: None,
        }
    }

    /// Creates a new block using the pending transactions from the
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

        self.payload_attributes_job = Some(Box::pin(async move {
            build_payload_attributes(database, max_l1_messages, payload_attributes).await
        }));

        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    /// Handle a reorg event.
    pub fn handle_reorg(&mut self, queue_index: u64, block_number: u64) {
        self.provider.set_index_cursor(queue_index);
        self.provider.set_l1_head(block_number);
    }

    /// Handle a new L1 block.
    pub fn handle_new_l1_block(&mut self, block_number: u64) {
        self.provider.set_l1_head(block_number);
    }
}

async fn build_payload_attributes<
    SMP: SequencerL1MessageProvider + Unpin + Send + Sync + 'static,
>(
    provider: Arc<SMP>,
    max_l1_messages: u64,
    payload_attributes: PayloadAttributes,
) -> Result<ScrollPayloadAttributes, SequencerError> {
    // Collect L1 messages to include in payload.
    let mut l1_messages = vec![];
    for _ in 0..max_l1_messages {
        match provider.next_l1_message().await.map_err(Into::into)? {
            Some(l1_message) => {
                l1_messages.push(l1_message.encoded_2718().into());
            }
            None => {
                break;
            }
        }
    }

    Ok(ScrollPayloadAttributes {
        payload_attributes,
        transactions: (!l1_messages.is_empty()).then_some(l1_messages),
        no_tx_pool: false,
        block_data_hint: None,
    })
}

impl<SMP> std::fmt::Debug for Sequencer<SMP> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sequencer")
            .field("provider", &"SequencerMessageProvider")
            .field("fee_recipient", &self.fee_recipient)
            .field("payload_building_job", &"PayloadBuildingJob")
            .field("l1_message_per_block", &self.max_l1_messages_per_block)
            .finish()
    }
}

impl<SMP> Stream for Sequencer<SMP> {
    type Item = ScrollPayloadAttributes;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(payload_building_job) = self.payload_attributes_job.as_mut() {
            match payload_building_job.as_mut().poll(cx) {
                Poll::Ready(Ok(block)) => {
                    self.payload_attributes_job = None;
                    Poll::Ready(Some(block))
                }
                Poll::Ready(Err(_)) => {
                    self.payload_attributes_job = None;
                    Poll::Ready(None)
                }
                Poll::Pending => Poll::Pending,
            }
        } else {
            self.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
