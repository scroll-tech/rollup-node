//! This library contains the sequencer, which is responsible for sequencing transactions and
//! producing new blocks.

use std::{future::Future, pin::Pin, sync::Arc};

use alloy_rpc_types_engine::{ForkchoiceState, PayloadAttributes};
use futures::Stream;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_providers::ExecutionPayloadProvider;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_db::{Database, DatabaseOperations};
use scroll_engine::EngineDriver;
use std::task::{Context, Poll};

mod error;
pub use error::SequencerError;

/// A type alias for the payload building job future.
pub type PayloadBuildingJobFuture =
    Pin<Box<dyn Future<Output = Result<(ScrollBlock, u64), SequencerError>> + Send>>;

/// The sequencer is responsible for sequencing transactions and producing new blocks.
pub struct Sequencer<EC, P> {
    /// A reference to the database
    database: Arc<Database>,
    /// The engine API
    engine: Arc<EngineDriver<EC, P>>,
    // TODO: The configuration below should be removed and instead replaced by a Provider type that
    // can yield L1 messages at the appropriate depth.
    /// The current L1 block number
    l1_block_number: u64,
    /// The current block number
    l2_block_number: u64,
    /// The next L1 message index
    l1_message_index: u64,
    /// The number of L1 blocks to wait for before including a L1 message in a block
    l1_message_delay: u64,
    /// The number of L1 messages to include in each block.
    max_l1_messages_per_block: u64,
    /// The inflight payload building job
    payload_building_job: Option<PayloadBuildingJobFuture>,
}

impl<EC, P> Sequencer<EC, P>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
{
    /// Creates a new sequencer.
    pub fn new(
        database: Arc<Database>,
        engine: Arc<EngineDriver<EC, P>>,
        l1_block_number: u64,
        l2_block_number: u64,
        l1_message_index: u64,
        l1_message_delay: u64,
        max_l1_messages_per_block: u64,
    ) -> Self {
        Self {
            database,
            engine,
            l1_block_number,
            l2_block_number,
            l1_message_index,
            l1_message_delay,
            max_l1_messages_per_block,
            payload_building_job: None,
        }
    }

    /// Creates a new block using the pending transactions from the
    pub fn build_block(&mut self, fcs: ForkchoiceState) {
        let payload_attributes = PayloadAttributes { timestamp: 1, ..Default::default() };
        let l1_message_start_index = self.l1_message_index;
        let max_l1_messages = self.max_l1_messages_per_block;

        let engine = self.engine.clone();
        let database = self.database.clone();

        self.payload_building_job = Some(Box::pin(async move {
            build_block(
                engine,
                database,
                l1_message_start_index,
                max_l1_messages,
                fcs,
                payload_attributes,
            )
            .await
        }));
    }

    /// Handle a reorg event.
    pub fn handle_reorg(&mut self, _block_number: u64) {
        todo!()
    }
}

async fn build_block<
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
>(
    engine: Arc<EngineDriver<EC, P>>,
    database: Arc<Database>,
    mut l1_message_start_index: u64,
    max_l1_messages: u64,
    fcs: ForkchoiceState,
    payload_attributes: PayloadAttributes,
) -> Result<(ScrollBlock, u64), SequencerError> {
    let mut l1_messages = vec![];

    loop {
        if l1_messages.len() == max_l1_messages as usize {
            break;
        }
        match database.get_l1_message(l1_message_start_index).await? {
            Some(l1_message) => {
                l1_messages.push(l1_message);
                l1_message_start_index += 1;
            }
            None => break,
        }
    }

    let scroll_payload_attributes =
        ScrollPayloadAttributes { payload_attributes, transactions: None, no_tx_pool: false };
    Ok((engine.build_new_payload(fcs, scroll_payload_attributes).await?, l1_message_start_index))
}

impl<EC, P> std::fmt::Debug for Sequencer<EC, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sequencer")
            .field("l1_block_number", &self.l1_block_number)
            .field("l2_block_number", &self.l2_block_number)
            .field("l1_message_index", &self.l1_message_index)
            .field("l1_message_delay", &self.l1_message_delay)
            .field("l1_message_per_block", &self.max_l1_messages_per_block)
            .finish()
    }
}

impl<EC, P> Stream for Sequencer<EC, P> {
    type Item = ScrollBlock;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(payload_building_job) = self.payload_building_job.as_mut() {
            match payload_building_job.as_mut().poll(cx) {
                Poll::Ready(Ok((block, l1_message_index))) => {
                    self.payload_building_job = None;
                    self.l1_message_index = l1_message_index;
                    Poll::Ready(Some(block))
                }
                Poll::Ready(Err(_)) => {
                    self.payload_building_job = None;
                    Poll::Ready(None)
                }
                Poll::Pending => Poll::Pending,
            }
        } else {
            Poll::Pending
        }
    }
}
