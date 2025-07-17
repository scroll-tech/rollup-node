//! A stateless derivation pipeline for Scroll.
//!
//! This crate provides a simple implementation of a derivation pipeline that transforms a batch
//! into payload attributes for block building.

mod data_source;

mod error;
pub use error::DerivationPipelineError;

mod metrics;
pub use metrics::DerivationPipelineMetrics;

use crate::data_source::CodecDataSource;
use std::{boxed::Box, collections::VecDeque, fmt::Formatter, sync::Arc, time::Instant, vec::Vec};

use alloy_primitives::{Address, B256};
use alloy_rpc_types_engine::PayloadAttributes;
use core::{
    fmt::Debug,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};
use futures::{FutureExt, Stream};
use rollup_node_primitives::{
    BatchCommitData, BatchInfo, ScrollPayloadAttributesWithBatchInfo, WithBlockNumber,
};
use rollup_node_providers::{BlockDataProvider, L1Provider};
use scroll_alloy_rpc_types_engine::{BlockDataHint, ScrollPayloadAttributes};
use scroll_codec::Codec;
use scroll_db::{Database, DatabaseOperations};

/// A future that resolves to a stream of [`ScrollPayloadAttributesWithBatchInfo`].
type DerivationPipelineFuture = Pin<
    Box<
        dyn Future<
                Output = Result<
                    Vec<ScrollPayloadAttributesWithBatchInfo>,
                    (Arc<BatchInfo>, DerivationPipelineError),
                >,
            > + Send,
    >,
>;

/// A structure holding the current unresolved futures for the derivation pipeline.
pub struct DerivationPipeline<P> {
    /// The current derivation pipeline futures polled.
    pipeline_future: Option<WithBlockNumber<DerivationPipelineFuture>>,
    /// A reference to the database.
    database: Arc<Database>,
    /// A L1 provider.
    l1_provider: P,
    /// The queue of batches to handle.
    batch_queue: VecDeque<WithBlockNumber<Arc<BatchInfo>>>,
    /// The queue of polled attributes.
    attributes_queue: VecDeque<WithBlockNumber<ScrollPayloadAttributesWithBatchInfo>>,
    /// The waker for the pipeline.
    waker: Option<Waker>,
    /// The metrics of the pipeline.
    metrics: DerivationPipelineMetrics,
}

impl<P: Debug> Debug for DerivationPipeline<P> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DerivationPipeline")
            .field(
                "pipeline_future",
                &self.pipeline_future.as_ref().map(|_| "Some( ... )").unwrap_or("None"),
            )
            .field("database", &self.database)
            .field("l1_provider", &self.l1_provider)
            .field("batch_queue", &self.batch_queue)
            .field("attributes_queue", &self.attributes_queue)
            .field("waker", &self.waker)
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl<P> DerivationPipeline<P>
where
    P: L1Provider + Clone + Send + Sync + 'static,
{
    /// Returns a new instance of the [`DerivationPipeline`].
    pub fn new(l1_provider: P, database: Arc<Database>) -> Self {
        Self {
            database,
            l1_provider,
            batch_queue: Default::default(),
            pipeline_future: None,
            attributes_queue: Default::default(),
            waker: None,
            metrics: DerivationPipelineMetrics::default(),
        }
    }

    /// Handles a new batch commit index by pushing it in its internal queue.
    /// Wakes the waker in order to trigger a call to poll.
    pub fn handle_batch_commit(&mut self, batch_info: BatchInfo, l1_block_number: u64) {
        let block_info = Arc::new(batch_info);
        self.batch_queue.push_back(WithBlockNumber::new(l1_block_number, block_info));
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }

    /// Handles the next batch index in the batch index queue, pushing the future in the pipeline
    /// futures.
    fn handle_next_batch(&mut self) -> Option<WithBlockNumber<DerivationPipelineFuture>> {
        let database = self.database.clone();
        let metrics = self.metrics.clone();
        let provider = self.l1_provider.clone();

        if let Some(info) = self.batch_queue.pop_front() {
            let block_number = info.number;
            let fut = Box::pin(async move {
                let derive_start = Instant::now();

                // get the batch commit data.
                let index = info.inner.index;
                let info = info.inner;
                let batch = database
                    .get_batch_by_index(index)
                    .await
                    .map_err(|err| (info.clone(), err.into()))?
                    .ok_or((info.clone(), DerivationPipelineError::UnknownBatch(index)))?;

                // derive the attributes and attach the corresponding batch info.
                let attrs =
                    derive(batch, provider, database).await.map_err(|err| (info.clone(), err))?;

                // update metrics.
                metrics.derived_blocks.increment(attrs.len() as u64);
                let execution_duration = derive_start.elapsed().as_secs_f64();
                metrics.blocks_per_second.set(attrs.len() as f64 / execution_duration);

                Ok(attrs.into_iter().map(|attr| (attr, *info).into()).collect())
            });
            return Some(WithBlockNumber::new(block_number, fut));
        }
        None
    }

    /// Clear attributes, batches and future for which the associated block number >
    /// `l1_block_number`.
    pub fn handle_reorg(&mut self, l1_block_number: u64) {
        self.batch_queue.retain(|batch| batch.number <= l1_block_number);
        if let Some(fut) = &mut self.pipeline_future {
            if fut.number > l1_block_number {
                self.pipeline_future = None;
            }
        }
        self.attributes_queue.retain(|attr| attr.number <= l1_block_number);
    }

    /// Flushes all the data in the pipeline.
    pub fn flush(&mut self) {
        self.attributes_queue.clear();
        self.batch_queue.clear();
        self.pipeline_future = None;
    }
}

impl<P> Stream for DerivationPipeline<P>
where
    P: L1Provider + Clone + Unpin + Send + Sync + 'static,
{
    type Item = ScrollPayloadAttributesWithBatchInfo;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // return attributes from the queue if any.
        if let Some(attribute) = this.attributes_queue.pop_front() {
            return Poll::Ready(Some(attribute.inner))
        }

        // if future is None and the batch queue is empty, store the waker and return.
        if this.pipeline_future.is_none() && this.batch_queue.is_empty() {
            this.waker = Some(cx.waker().clone());
            return Poll::Pending
        }

        // if the future is None, handle the next batch.
        if this.pipeline_future.is_none() {
            this.pipeline_future = this.handle_next_batch()
        }

        // poll the futures and handle result.
        if let Some(Poll::Ready(res)) = this.pipeline_future.as_mut().map(|fut| fut.poll_unpin(cx))
        {
            match res {
                WithBlockNumber { inner: Ok(attributes), number } => {
                    let attributes =
                        attributes.into_iter().map(|attr| WithBlockNumber::new(number, attr));
                    this.attributes_queue.extend(attributes);
                    this.pipeline_future = None;
                    cx.waker().wake_by_ref();
                }
                WithBlockNumber { inner: Err((batch_info, err)), number } => {
                    tracing::error!(target: "scroll::node::derivation_pipeline", batch_info = ?*batch_info, ?err, "failed to derive payload attributes for batch");
                    // retry polling the same batch.
                    this.batch_queue.push_front(WithBlockNumber::new(number, batch_info));
                    let fut = this.handle_next_batch().expect("Pushed batch info into queue");
                    this.pipeline_future = Some(fut)
                }
            }
        }
        Poll::Pending
    }
}

/// Returns a vector of [`ScrollPayloadAttributes`] from the [`BatchCommitData`] and a
/// [`L1Provider`].
pub async fn derive<L1P: L1Provider + Sync + Send, L2P: BlockDataProvider + Sync + Send>(
    batch: BatchCommitData,
    l1_provider: L1P,
    l2_provider: L2P,
) -> Result<Vec<ScrollPayloadAttributes>, DerivationPipelineError> {
    // fetch the blob then decode the input batch.
    let blob = if let Some(hash) = batch.blob_versioned_hash {
        l1_provider.blob(batch.block_timestamp, hash).await?
    } else {
        None
    };
    let data = CodecDataSource { calldata: batch.calldata.as_ref(), blob: blob.as_deref() };
    let decoded = Codec::decode(&data)?;

    // set the cursor for the l1 provider.
    let data = &decoded.data;
    if let Some(index) = data.queue_index_start() {
        l1_provider.set_queue_index_cursor(index);
    } else if let Some(hash) = data.prev_l1_message_queue_hash() {
        l1_provider.set_hash_cursor(*hash).await;
        // we skip the first l1 message, as we are interested in the one starting after
        // prev_l1_message_queue_hash.
        let _ = l1_provider.next_l1_message().await.map_err(Into::into)?;
    } else {
        return Err(DerivationPipelineError::MissingL1MessageQueueCursor)
    }

    let skipped_l1_messages = decoded.data.skipped_l1_message_bitmap.clone().unwrap_or_default();
    let mut skipped_l1_messages = skipped_l1_messages.into_iter();
    let blocks = decoded.data.into_l2_blocks();
    let mut attributes = Vec::with_capacity(blocks.len());
    for mut block in blocks {
        // query the appropriate amount of l1 messages.
        let mut txs = Vec::with_capacity(block.context.num_transactions as usize);
        for _ in 0..block.context.num_l1_messages {
            // check if the next l1 message should be skipped.
            if matches!(skipped_l1_messages.next(), Some(bit) if bit == 1) {
                l1_provider.increment_cursor();
                continue;
            }

            // TODO: fetch L1 messages range.
            let l1_message = l1_provider
                .next_l1_message()
                .await
                .map_err(Into::into)?
                .ok_or(DerivationPipelineError::MissingL1Message(block.clone()))?;
            let mut bytes = Vec::with_capacity(l1_message.eip2718_encoded_length());
            l1_message.eip2718_encode(&mut bytes);
            txs.push(bytes.into());
        }

        // add the block transactions.
        txs.append(&mut block.transactions);

        // get the block data for the l2 block.
        let number = block.context.number;
        // TODO(performance): can this be improved by adding block_data_range.
        let block_data = l2_provider.block_data(number).await.map_err(Into::into)?;

        // construct the payload attributes.
        let attribute = ScrollPayloadAttributes {
            payload_attributes: PayloadAttributes {
                timestamp: block.context.timestamp,
                suggested_fee_recipient: Address::ZERO,
                prev_randao: B256::ZERO,
                withdrawals: None,
                parent_beacon_block_root: None,
            },
            transactions: Some(txs),
            no_tx_pool: true,
            block_data_hint: block_data.unwrap_or_else(BlockDataHint::none),
            gas_limit: Some(block.context.gas_limit),
        };
        attributes.push(attribute);
    }

    Ok(attributes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use alloy_eips::{eip4844::Blob, Decodable2718};
    use alloy_primitives::{address, b256, bytes, U256};
    use core::sync::atomic::{AtomicU64, Ordering};
    use futures::StreamExt;
    use rollup_node_primitives::L1MessageEnvelope;
    use rollup_node_providers::{
        DatabaseL1MessageProvider, L1BlobProvider, L1MessageProvider, L1ProviderError,
    };
    use scroll_alloy_consensus::TxL1Message;
    use scroll_alloy_rpc_types_engine::BlockDataHint;
    use scroll_codec::decoding::test_utils::read_to_bytes;
    use scroll_db::test_utils::setup_test_db;

    struct MockL1MessageProvider {
        messages: Arc<Vec<L1MessageEnvelope>>,
        index: AtomicU64,
    }

    struct Infallible;
    impl From<Infallible> for L1ProviderError {
        fn from(_value: Infallible) -> Self {
            Self::Other("infallible")
        }
    }

    #[async_trait::async_trait]
    impl L1BlobProvider for MockL1MessageProvider {
        async fn blob(
            &self,
            _block_timestamp: u64,
            _hash: B256,
        ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl L1MessageProvider for MockL1MessageProvider {
        type Error = Infallible;

        async fn get_l1_message_with_block_number(
            &self,
        ) -> Result<Option<L1MessageEnvelope>, Self::Error> {
            let index = self.index.load(Ordering::Relaxed);
            Ok(self.messages.get(index as usize).cloned())
        }

        fn set_queue_index_cursor(&self, _index: u64) {}

        async fn set_hash_cursor(&self, _hash: B256) {}

        fn increment_cursor(&self) {
            self.index.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Clone)]
    struct MockL1Provider<P: L1MessageProvider> {
        l1_messages_provider: P,
    }

    #[async_trait::async_trait]
    impl<P: L1MessageProvider + Sync> L1BlobProvider for MockL1Provider<P> {
        async fn blob(
            &self,
            _block_timestamp: u64,
            _hash: B256,
        ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl<P: L1MessageProvider + Send + Sync> L1MessageProvider for MockL1Provider<P> {
        type Error = P::Error;

        async fn get_l1_message_with_block_number(
            &self,
        ) -> Result<Option<L1MessageEnvelope>, Self::Error> {
            self.l1_messages_provider.get_l1_message_with_block_number().await
        }
        fn set_queue_index_cursor(&self, index: u64) {
            self.l1_messages_provider.set_queue_index_cursor(index);
        }
        async fn set_hash_cursor(&self, hash: B256) {
            self.l1_messages_provider.set_hash_cursor(hash).await
        }
        fn increment_cursor(&self) {
            self.l1_messages_provider.increment_cursor()
        }
    }

    struct MockL2Provider;

    #[async_trait::async_trait]
    impl BlockDataProvider for MockL2Provider {
        type Error = Infallible;

        async fn block_data(
            &self,
            _block_number: u64,
        ) -> Result<Option<BlockDataHint>, Self::Error> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn test_should_stream_payload_attributes() -> eyre::Result<()> {
        // https://etherscan.io/tx/0x8f4f0fcab656aa81589db5b53255094606c4624bfd99702b56b2debaf6211f48
        // load batch data in the db.
        let db = Arc::new(setup_test_db().await);
        let raw_calldata = read_to_bytes("./testdata/calldata_v0.bin")?;
        let batch_data = BatchCommitData {
            hash: b256!("7f26edf8e3decbc1620b4d2ba5f010a6bdd10d6bb16430c4f458134e36ab3961"),
            index: 12,
            block_number: 18319648,
            block_timestamp: 1696935971,
            calldata: Arc::new(raw_calldata),
            blob_versioned_hash: None,
            finalized_block_number: None,
        };
        db.insert_batch(batch_data).await?;
        // load messages in db.
        let l1_messages = vec![
            L1MessageEnvelope {
                l1_block_number: 717,
                l2_block_number: None,
                queue_hash: None,
                transaction: TxL1Message {
                    queue_index: 33,
                    gas_limit: 168000,
                    to: address!("781e90f1c8Fc4611c9b7497C3B47F99Ef6969CbC"),
                    value: U256::ZERO,
                    sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
                    input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000006a94d74f430000000000000000000000000000000000000000000000000000000000000000002100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000000000000000000000000000006a94d74f4300000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
            },
            L1MessageEnvelope {
                l1_block_number: 717,
                l2_block_number: None,
                queue_hash: None,
                transaction: TxL1Message {
                    queue_index: 34,
                    gas_limit: 168000,
                    to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
                    value: U256::ZERO,
                    sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
                    input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf000000000000000000000000000000000000000000000000000470de4df820000000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f00000000000000000000000000000000000000000000000000470de4df8200000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
            },
        ];
        for message in l1_messages {
            db.insert_l1_message(message).await?;
        }

        // construct the pipeline.
        let l1_messages_provider = DatabaseL1MessageProvider::new(db.clone(), 0);
        let mock_l1_provider = MockL1Provider { l1_messages_provider };
        let mut pipeline = DerivationPipeline::new(mock_l1_provider, db);

        // as long as we don't call `handle_commit_batch`, pipeline should not return attributes.
        pipeline.handle_batch_commit(BatchInfo { index: 12, hash: Default::default() }, 0);

        // we should find some attributes now
        assert!(pipeline.next().await.is_some());

        // check the correctness of the last attribute.
        let mut attribute = ScrollPayloadAttributes::default();
        while let Some(ScrollPayloadAttributesWithBatchInfo { payload_attributes: a, .. }) =
            pipeline.next().await
        {
            if a.payload_attributes.timestamp == 1696935657 {
                attribute = a;
                break
            }
        }
        let expected = ScrollPayloadAttributes {
            payload_attributes: PayloadAttributes {
                timestamp: 1696935657,
                ..Default::default()
            },
            transactions: Some(vec![bytes!("f88c8202658417d7840082a4f294530000000000000000000000000000000000000280a4bede39b500000000000000000000000000000000000000000000000000000001669aa2f583104ec4a07461e6555f927393ebdf5f183738450c3842bc3b86a1db7549d9bee21fadd0b1a06d7ba96897bd9fb8e838a327d3ca34be66da11955f10d1fb2264949071e9e8cd")]),
            no_tx_pool: true,
            block_data_hint: BlockDataHint::none(),
            gas_limit: Some(10_000_000),
        };
        assert_eq!(attribute, expected);

        Ok(())
    }

    #[tokio::test]
    async fn test_should_derive_batch() -> eyre::Result<()> {
        // https://etherscan.io/tx/0x8f4f0fcab656aa81589db5b53255094606c4624bfd99702b56b2debaf6211f48
        let raw_calldata = read_to_bytes("./testdata/calldata_v0.bin")?;
        let batch_data = BatchCommitData {
            hash: b256!("7f26edf8e3decbc1620b4d2ba5f010a6bdd10d6bb16430c4f458134e36ab3961"),
            index: 12,
            block_number: 18319648,
            block_timestamp: 1696935971,
            calldata: Arc::new(raw_calldata),
            blob_versioned_hash: None,
            finalized_block_number: None,
        };

        let l1_messages = vec![
            L1MessageEnvelope {
                l1_block_number: 5,
                l2_block_number: None,
                queue_hash: None,
                transaction: TxL1Message {
                    queue_index: 33,
                    gas_limit: 168000,
                    to: address!("781e90f1c8Fc4611c9b7497C3B47F99Ef6969CbC"),
                    value: U256::ZERO,
                    sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
                    input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000006a94d74f430000000000000000000000000000000000000000000000000000000000000000002100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000000000000000000000000000006a94d74f4300000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
            },
            L1MessageEnvelope {
                l1_block_number: 10,
                l2_block_number: None,
                queue_hash: None,
                transaction: TxL1Message {
                    queue_index: 34,
                    gas_limit: 168000,
                    to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
                    value: U256::ZERO,
                    sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
                    input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf000000000000000000000000000000000000000000000000000470de4df820000000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f00000000000000000000000000000000000000000000000000470de4df8200000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
            },
        ];
        let l1_provider =
            MockL1MessageProvider { messages: Arc::new(l1_messages), index: 0.into() };
        let l2_provider = MockL2Provider;

        let attributes: Vec<_> = derive(batch_data, l1_provider, l2_provider).await?;
        let attribute =
            attributes.iter().find(|a| a.payload_attributes.timestamp == 1696935384).unwrap();

        let expected = ScrollPayloadAttributes {
            payload_attributes: PayloadAttributes {
                timestamp: 1696935384,
                ..Default::default()
            },
            transactions: Some(vec![bytes!("7ef901b7218302904094781e90f1c8fc4611c9b7497c3b47f99ef6969cbc80b901848ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000006a94d74f430000000000000000000000000000000000000000000000000000000000000000002100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000000000000000000000000000006a94d74f4300000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000947885bcbd5cecef1336b5300fb5186a12ddd8c478"), bytes!("7ef901b7228302904094781e90f1c8fc4611c9b7497c3b47f99ef6969cbc80b901848ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf000000000000000000000000000000000000000000000000000470de4df820000000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f00000000000000000000000000000000000000000000000000470de4df8200000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000947885bcbd5cecef1336b5300fb5186a12ddd8c478")]),
            no_tx_pool: true,
            block_data_hint: BlockDataHint::none(),
            gas_limit: Some(10_000_000),
        };
        assert_eq!(attribute, &expected);

        let attribute = attributes.last().unwrap();
        let expected = ScrollPayloadAttributes {
            payload_attributes: PayloadAttributes {
                timestamp: 1696935657,
                ..Default::default()
            },
            transactions: Some(vec![bytes!("f88c8202658417d7840082a4f294530000000000000000000000000000000000000280a4bede39b500000000000000000000000000000000000000000000000000000001669aa2f583104ec4a07461e6555f927393ebdf5f183738450c3842bc3b86a1db7549d9bee21fadd0b1a06d7ba96897bd9fb8e838a327d3ca34be66da11955f10d1fb2264949071e9e8cd")]),
            no_tx_pool: true,
            block_data_hint: BlockDataHint::none(),
            gas_limit: Some(10_000_000),
        };
        assert_eq!(attribute, &expected);

        Ok(())
    }

    #[tokio::test]
    async fn test_should_skip_l1_messages() -> eyre::Result<()> {
        // https://sepolia.etherscan.io/tx/0xe9d7a634a2afd8adee5deab180c30d261e05fea499ccbfd5c987436fe587850e
        let raw_calldata = read_to_bytes("./testdata/calldata_v0_with_skipped_l1_messages.bin")?;
        let batch_data = BatchCommitData {
            hash: b256!("1e86131f4204278feb116e3043916c6bd598b1b092b550e236edb2e4a398730a"),
            index: 100,
            block_number: 4045729,
            block_timestamp: 1691454067,
            calldata: Arc::new(raw_calldata),
            blob_versioned_hash: None,
            finalized_block_number: None,
        };

        // prepare the l1 messages.
        let l1_messages = vec![
            L1MessageEnvelope {
                l1_block_number: 5,
                l2_block_number: None,
                queue_hash: None,
                transaction: TxL1Message {
                    queue_index: 19,
                    gas_limit: 1000000,
                    to: address!("bA50F5340fb9f3bD074Bd638C9be13Ecb36e603D"),
                    value: U256::ZERO,
                    sender: address!("61d8d3E7F7c656493d1d76aAA1a836CEdfCBc27b"),
                    input: bytes!("8ef1332e0000000000000000000000008a54a2347da2562917304141ab67324615e9866d00000000000000000000000091e8addfe1358aca5314c644312d38237fc1101c000000000000000000000000000000000000000000000000016345785d8a0000000000000000000000000000000000000000000000000000000000000000001400000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e874800000000000000000000000098110937b5d6c5fcb0ba99480e585d2364e9809c00000000000000000000000098110937b5d6c5fcb0ba99480e585d2364e9809c000000000000000000000000000000000000000000000000016345785d8a00000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
            },
            L1MessageEnvelope {
                l1_block_number: 5,
                l2_block_number: None,
                queue_hash: None,
                transaction: TxL1Message {
                    queue_index: 20,
                    gas_limit: 400000,
                    to: address!("bA50F5340fb9f3bD074Bd638C9be13Ecb36e603D"),
                    value: U256::ZERO,
                    sender: address!("61d8d3E7F7c656493d1d76aAA1a836CEdfCBc27b"),
                    input: bytes!("8ef1332e0000000000000000000000008a54a2347da2562917304141ab67324615e9866d00000000000000000000000091e8addfe1358aca5314c644312d38237fc1101c000000000000000000000000000000000000000000000000016345785d8a0000000000000000000000000000000000000000000000000000000000000000001400000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e874800000000000000000000000098110937b5d6c5fcb0ba99480e585d2364e9809c00000000000000000000000098110937b5d6c5fcb0ba99480e585d2364e9809c000000000000000000000000000000000000000000000000016345785d8a00000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
            },
            L1MessageEnvelope {
                l1_block_number: 10,
                l2_block_number: None,
                queue_hash: None,
                transaction: TxL1Message {
                    queue_index: 21,
                    gas_limit: 400000,
                    to: address!("bA50F5340fb9f3bD074Bd638C9be13Ecb36e603D"),
                    value: U256::ZERO,
                    sender: address!("61d8d3E7F7c656493d1d76aAA1a836CEdfCBc27b"),
                    input: bytes!("8ef1332e0000000000000000000000008a54a2347da2562917304141ab67324615e9866d00000000000000000000000091e8addfe1358aca5314c644312d38237fc1101c0000000000000000000000000000000000000000000000004563918244f40000000000000000000000000000000000000000000000000000000000000000001500000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e87480000000000000000000000004721cf824b6750b58d781fd1336d92a082704c7a0000000000000000000000004721cf824b6750b58d781fd1336d92a082704c7a0000000000000000000000000000000000000000000000004563918244f400000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
            },
        ];
        let l1_provider =
            MockL1MessageProvider { messages: Arc::new(l1_messages.clone()), index: 0.into() };
        let l2_provider = MockL2Provider;

        // derive attributes and extract l1 messages.
        let attributes: Vec<_> = derive(batch_data, l1_provider, l2_provider).await?;
        let derived_l1_messages: Vec<_> = attributes
            .into_iter()
            .filter_map(|a| a.transactions)
            .flatten()
            .filter_map(|rlp| {
                let buf = &mut rlp.as_ref();
                TxL1Message::decode_2718(buf).ok()
            })
            .collect();

        // the first L1 message should be skipped.
        let expected_l1_messages: Vec<_> =
            l1_messages[1..].iter().map(|msg| msg.transaction.clone()).collect();
        assert_eq!(expected_l1_messages, derived_l1_messages);
        Ok(())
    }

    async fn new_test_pipeline(
    ) -> DerivationPipeline<MockL1Provider<DatabaseL1MessageProvider<Arc<Database>>>> {
        let initial_block = 200;

        let batches = (initial_block - 100..initial_block)
            .map(|i| WithBlockNumber::new(i, Arc::new(BatchInfo::new(i, B256::random()))));
        let attributes = (initial_block..initial_block + 100)
            .zip(batches.clone())
            .map(|(i, batch)| {
                WithBlockNumber::new(
                    i,
                    ScrollPayloadAttributesWithBatchInfo {
                        batch_info: *batch.inner,
                        ..Default::default()
                    },
                )
            })
            .collect();

        let db = Arc::new(setup_test_db().await);
        let l1_messages_provider = DatabaseL1MessageProvider::new(db.clone(), 0);
        let mock_l1_provider = MockL1Provider { l1_messages_provider };

        DerivationPipeline {
            pipeline_future: Some(WithBlockNumber::new(
                initial_block,
                Box::pin(async { Ok(vec![]) }),
            )),
            database: db,
            l1_provider: mock_l1_provider,
            batch_queue: batches.collect(),
            attributes_queue: attributes,
            waker: None,
            metrics: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_should_handle_reorgs() -> eyre::Result<()> {
        // set up pipeline.
        let mut pipeline = new_test_pipeline().await;

        // reorg at block 0.
        pipeline.handle_reorg(0);
        // should completely clear the pipeline.
        assert!(pipeline.batch_queue.is_empty());
        assert!(pipeline.pipeline_future.is_none());
        assert!(pipeline.attributes_queue.is_empty());

        // set up pipeline.
        let mut pipeline = new_test_pipeline().await;

        // reorg at block 200.
        pipeline.handle_reorg(200);
        // should clear all but one attribute and retain all batches and the pending future.
        assert_eq!(pipeline.batch_queue.len(), 100);
        assert!(pipeline.pipeline_future.is_some());
        assert_eq!(pipeline.attributes_queue.len(), 1);

        // set up pipeline.
        let mut pipeline = new_test_pipeline().await;

        // reorg at block 300.
        pipeline.handle_reorg(300);
        // should retain all batches, attributes and the pending future.
        assert_eq!(pipeline.batch_queue.len(), 100);
        assert!(pipeline.pipeline_future.is_some());
        assert_eq!(pipeline.attributes_queue.len(), 100);

        Ok(())
    }
}
