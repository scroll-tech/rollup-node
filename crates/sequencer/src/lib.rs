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
use alloy_rpc_types_engine::{ForkchoiceState, PayloadAttributes};
use futures::Stream;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_providers::{
    DatabaseL1MessageDelayProvider, ExecutionPayloadProvider, L1MessageProvider,
};
use scroll_alloy_provider::ScrollEngineApi;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_db::{Database, DatabaseConnectionProvider, DatabaseOperations};
use scroll_engine::EngineDriver;
use std::task::{Context, Poll};

mod error;
pub use error::SequencerError;

/// A type alias for the payload building job future.
pub type PayloadBuildingJobFuture =
    Pin<Box<dyn Future<Output = Result<(ScrollBlock, u64), SequencerError>> + Send>>;

/// The sequencer is responsible for sequencing transactions and producing new blocks.
pub struct Sequencer<DB, EC, P> {
    /// A reference to the database
    database: Arc<DatabaseL1MessageDelayProvider<DB>>,
    /// The engine API
    engine: Arc<EngineDriver<EC, P>>,
    /// The fee recipient
    fee_recipient: Address,
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

impl<DB, EC, P> Sequencer<DB, EC, P>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
    DB: DatabaseConnectionProvider + Unpin + Send + Sync + 'static,
{
    /// Creates a new sequencer.
    pub fn new(
        database: Arc<DatabaseL1MessageDelayProvider<DB>>,
        engine: Arc<EngineDriver<EC, P>>,
        fee_recipient: Address,
        l1_block_number: u64,
        l2_block_number: u64,
        l1_message_index: u64,
        l1_message_delay: u64,
        max_l1_messages_per_block: u64,
    ) -> Self {
        Self {
            database,
            engine,
            fee_recipient,
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
        if self.payload_building_job.is_some() {
            tracing::warn!(target: "rollup_node::sequender", "A payload building job is already in progress");
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
    DB: DatabaseConnectionProvider + Unpin + Send + Sync + 'static,
>(
    engine: Arc<EngineDriver<EC, P>>,
    mut provider: Arc<DatabaseL1MessageDelayProvider<DB>>,
    mut l1_message_start_index: u64,
    max_l1_messages: u64,
    fcs: ForkchoiceState,
    payload_attributes: PayloadAttributes,
) -> Result<(ScrollBlock, u64), SequencerError> {
    let mut l1_messages = vec![];

    loop {
        if l1_messages.len() == max_l1_messages as usize {
            println!("breaking due to max length");
            break;
        }
        match provider.next_l1_message().await? {
            Some(l1_message) => {
                l1_messages.push(l1_message);
            }
            None => {
                println!("breaking as no messages yielded");
                break;
            }
        }
    }

    let transactions = l1_messages
        .into_iter()
        .map(|l1_message| l1_message.encoded_2718().into())
        .collect::<Vec<_>>();

    println!("transactions: {:?}", transactions);

    let scroll_payload_attributes = ScrollPayloadAttributes {
        payload_attributes,
        transactions: (!transactions.is_empty()).then_some(transactions),
        no_tx_pool: false,
    };
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
