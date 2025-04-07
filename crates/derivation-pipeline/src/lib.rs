//! A stateless derivation pipeline for Scroll.
//!
//! This crate provides a simple implementation of a derivation pipeline that transforms a batch
//! into payload attributes for block building.

#![cfg_attr(not(feature = "std"), no_std)]

mod data_source;

pub use error::DerivationPipelineError;
mod error;

#[cfg(not(feature = "std"))]
extern crate alloc as std;

use crate::data_source::CodecDataSource;
use std::{boxed::Box, collections::VecDeque, sync::Arc, vec::Vec};

use alloy_primitives::B256;
use alloy_rpc_types_engine::PayloadAttributes;
use core::{
    fmt::Debug,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};
use futures::{ready, stream::FuturesOrdered, Stream, StreamExt};
use reth_scroll_chainspec::SCROLL_FEE_VAULT_ADDRESS;
use rollup_node_primitives::BatchCommitData;
use rollup_node_providers::L1Provider;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_codec::Codec;
use scroll_db::{Database, DatabaseOperations};

/// A future that resolves to a stream of [`ScrollPayloadAttributes`].
type DerivationPipelineFuture = Pin<
    Box<
        dyn Future<Output = Result<Vec<ScrollPayloadAttributes>, (u64, DerivationPipelineError)>>
            + Send,
    >,
>;

/// Limit the amount of pipeline futures allowed to be polled concurrently.
const MAX_CONCURRENT_DERIVATION_PIPELINE_FUTS: usize = 20;

/// A structure holding the current unresolved futures for the derivation pipeline.
#[derive(Debug)]
pub struct DerivationPipeline<P> {
    /// The current derivation pipeline futures polled.
    pipeline_futures: FuturesOrdered<DerivationPipelineFuture>,
    /// A reference to the database.
    database: Arc<Database>,
    /// A L1 provider.
    l1_provider: P,
    /// The queue of batches to handle.
    batch_index_queue: VecDeque<u64>,
    /// The queue of polled attributes.
    attributes_queue: VecDeque<ScrollPayloadAttributes>,
    /// The waker for the pipeline.
    waker: Option<Waker>,
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
            batch_index_queue: Default::default(),
            pipeline_futures: Default::default(),
            attributes_queue: Default::default(),
            waker: None,
        }
    }

    /// Handles a new batch commit index by pushing it in its internal queue.
    /// Wakes the waker in order to trigger a call to poll.
    pub fn handle_batch_commit(&mut self, index: u64) {
        self.batch_index_queue.push_back(index);
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }

    /// Handles the next batch index in the batch index queue, pushing the future in the pipeline
    /// futures.
    fn handle_next_batch<
        F: FnMut(&mut FuturesOrdered<DerivationPipelineFuture>, DerivationPipelineFuture),
    >(
        &mut self,
        mut queue_fut: F,
    ) {
        let database = self.database.clone();
        let provider = self.l1_provider.clone();

        if let Some(index) = self.batch_index_queue.pop_front() {
            let fut = Box::pin(async move {
                let batch = database
                    .get_batch_by_index(index)
                    .await
                    .map_err(|err| (index, err.into()))?
                    .ok_or((index, DerivationPipelineError::UnknownBatch(index)))?;

                derive(batch, provider).await.map_err(|err| (index, err))
            });
            queue_fut(&mut self.pipeline_futures, fut);
        }
    }
}

impl<P> Stream for DerivationPipeline<P>
where
    P: L1Provider + Clone + Unpin + Send + Sync + 'static,
{
    type Item = ScrollPayloadAttributes;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // return attributes from the queue if any.
        if let Some(attribute) = this.attributes_queue.pop_front() {
            return Poll::Ready(Some(attribute))
        }

        // if futures are empty and the batch queue is empty, store the waker
        // and return.
        if this.pipeline_futures.is_empty() && this.batch_index_queue.is_empty() {
            this.waker = Some(cx.waker().clone());
            return Poll::Pending
        }

        // if the futures can still grow, handle the next batch.
        if this.pipeline_futures.len() < MAX_CONCURRENT_DERIVATION_PIPELINE_FUTS {
            this.handle_next_batch(|queue, fut| queue.push_back(fut));
        }

        // poll the futures and handle result.
        if let Some(res) = ready!(this.pipeline_futures.poll_next_unpin(cx)) {
            match res {
                Ok(attributes) => {
                    this.attributes_queue.extend(attributes);
                    cx.waker().wake_by_ref();
                }
                Err((index, err)) => {
                    tracing::error!(target: "scroll::node::derivation_pipeline", ?index, ?err, "failed to derive payload attributes for batch");
                    // retry polling the same batch index.
                    this.batch_index_queue.push_front(index);
                    this.handle_next_batch(|queue, fut| queue.push_front(fut));
                }
            }
        }
        Poll::Pending
    }
}

/// Returns a vector of [`ScrollPayloadAttributes`] from the [`BatchCommitData`] and a
/// [`L1Provider`].
pub async fn derive<P: L1Provider>(
    batch: BatchCommitData,
    l1_provider: P,
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
        l1_provider.set_index_cursor(index)
    } else if let Some(hash) = data.prev_l1_message_queue_hash() {
        l1_provider.set_hash_cursor(*hash);
        // we skip the first l1 message, as we are interested in the one starting after
        // prev_l1_message_queue_hash.
        let _ = l1_provider.next_l1_message().await.map_err(Into::into)?;
    } else {
        return Err(DerivationPipelineError::MissingL1MessageQueueCursor)
    }

    let blocks = decoded.data.into_l2_blocks();
    let mut attributes = Vec::with_capacity(blocks.len());
    for mut block in blocks {
        // query the appropriate amount of l1 messages.
        let mut txs = Vec::with_capacity(block.context.num_l1_messages as usize);
        for _ in 0..block.context.num_l1_messages {
            let l1_message = l1_provider
                .next_l1_message()
                .await
                .map_err(Into::into)?
                .ok_or(DerivationPipelineError::MissingL1Message)?;
            let mut bytes = Vec::with_capacity(l1_message.eip2718_encoded_length());
            l1_message.eip2718_encode(&mut bytes);
            txs.push(bytes.into());
        }

        // add the block transactions.
        txs.append(&mut block.transactions);

        // construct the payload attributes.
        let attribute = ScrollPayloadAttributes {
            payload_attributes: PayloadAttributes {
                timestamp: block.context.timestamp,
                // TODO: this should be based off the current configuration value.
                suggested_fee_recipient: SCROLL_FEE_VAULT_ADDRESS,
                prev_randao: B256::ZERO,
                withdrawals: None,
                parent_beacon_block_root: None,
            },
            transactions: Some(txs),
            no_tx_pool: true,
        };
        attributes.push(attribute);
    }

    Ok(attributes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use alloy_eips::eip4844::Blob;
    use alloy_primitives::{address, b256, bytes, U256};
    use futures::task::noop_waker_ref;
    use rollup_node_primitives::L1MessageWithBlockNumber;
    use rollup_node_providers::{
        DatabaseL1MessageProvider, L1BlobProvider, L1MessageProvider, L1ProviderError,
    };
    use scroll_alloy_consensus::TxL1Message;
    use scroll_codec::decoding::test_utils::read_to_bytes;
    use scroll_db::test_utils::setup_test_db;
    use tokio::sync::Mutex;

    struct MockL1MessageProvider {
        messages: Arc<Mutex<Vec<TxL1Message>>>,
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

        async fn next_l1_message(&self) -> Result<Option<TxL1Message>, Self::Error> {
            Ok(Some(self.messages.try_lock().expect("lock is free").remove(0)))
        }

        fn set_index_cursor(&self, _index: u64) {}

        fn set_hash_cursor(&self, _hash: B256) {}
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
    impl<P: L1MessageProvider + Sync> L1MessageProvider for MockL1Provider<P> {
        type Error = P::Error;

        async fn next_l1_message(&self) -> Result<Option<TxL1Message>, Self::Error> {
            self.l1_messages_provider.next_l1_message().await
        }
        fn set_index_cursor(&self, index: u64) {
            self.l1_messages_provider.set_index_cursor(index)
        }
        fn set_hash_cursor(&self, hash: B256) {
            self.l1_messages_provider.set_hash_cursor(hash)
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
        };
        db.insert_batch(batch_data).await?;

        // load messages in db.
        let l1_messages = vec![
            L1MessageWithBlockNumber{ block_number: 717, transaction: TxL1Message {
            queue_index: 33,
            gas_limit: 168000,
            to: address!("781e90f1c8Fc4611c9b7497C3B47F99Ef6969CbC"),
            value: U256::ZERO,
            sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
            input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000006a94d74f430000000000000000000000000000000000000000000000000000000000000000002100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000000000000000000000000000006a94d74f4300000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
        } }, L1MessageWithBlockNumber{transaction: TxL1Message {
            queue_index: 34,
            gas_limit: 168000,
            to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
            value: U256::ZERO,
            sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
            input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf000000000000000000000000000000000000000000000000000470de4df820000000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f00000000000000000000000000000000000000000000000000470de4df8200000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
        }, block_number: 717}];
        for message in l1_messages {
            db.insert_l1_message(message).await?;
        }

        // construct the pipeline.
        let l1_messages_provider = DatabaseL1MessageProvider::new(db.clone());
        let mock_l1_provider = MockL1Provider { l1_messages_provider };
        let mut pipeline = DerivationPipeline::new(mock_l1_provider, db);

        // as long as we don't call `handle_commit_batch`, pipeline should not return attributes.
        pipeline.handle_batch_commit(12);

        // we should find some attributes now
        assert!(pipeline.next().await.is_some());

        // check the correctness of the last attribute.
        let mut attribute = ScrollPayloadAttributes::default();
        while let Some(a) = pipeline.next().await {
            if a.payload_attributes.timestamp == 1696935657 {
                attribute = a;
                break
            }
        }
        let expected = ScrollPayloadAttributes{
            payload_attributes: PayloadAttributes{
                timestamp: 1696935657,
                suggested_fee_recipient: SCROLL_FEE_VAULT_ADDRESS,
                ..Default::default()
            },
            transactions: Some(vec![bytes!("f88c8202658417d7840082a4f294530000000000000000000000000000000000000280a4bede39b500000000000000000000000000000000000000000000000000000001669aa2f583104ec4a07461e6555f927393ebdf5f183738450c3842bc3b86a1db7549d9bee21fadd0b1a06d7ba96897bd9fb8e838a327d3ca34be66da11955f10d1fb2264949071e9e8cd")]),
            no_tx_pool: true,
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
        };

        let l1_messages = vec![TxL1Message {
            queue_index: 33,
            gas_limit: 168000,
            to: address!("781e90f1c8Fc4611c9b7497C3B47F99Ef6969CbC"),
            value: U256::ZERO,
            sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
            input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000006a94d74f430000000000000000000000000000000000000000000000000000000000000000002100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000000000000000000000000000006a94d74f4300000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
        },TxL1Message {
            queue_index: 34,
            gas_limit: 168000,
            to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
            value: U256::ZERO,
            sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
            input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf000000000000000000000000000000000000000000000000000470de4df820000000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f00000000000000000000000000000000000000000000000000470de4df8200000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
        }];
        let provider = MockL1MessageProvider { messages: Arc::new(Mutex::new(l1_messages)) };

        let attributes: Vec<_> = derive(batch_data, provider).await?;
        let attribute =
            attributes.iter().find(|a| a.payload_attributes.timestamp == 1696935384).unwrap();

        let expected = ScrollPayloadAttributes{
            payload_attributes: PayloadAttributes{
                timestamp: 1696935384,
                suggested_fee_recipient: SCROLL_FEE_VAULT_ADDRESS,
                ..Default::default()
            },
            transactions: Some(vec![bytes!("7ef901b7218302904094781e90f1c8fc4611c9b7497c3b47f99ef6969cbc80b901848ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000006a94d74f430000000000000000000000000000000000000000000000000000000000000000002100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000000000000000000000000000006a94d74f4300000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000947885bcbd5cecef1336b5300fb5186a12ddd8c478"), bytes!("7ef901b7228302904094781e90f1c8fc4611c9b7497c3b47f99ef6969cbc80b901848ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf000000000000000000000000000000000000000000000000000470de4df820000000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f00000000000000000000000000000000000000000000000000470de4df8200000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000947885bcbd5cecef1336b5300fb5186a12ddd8c478")]),
            no_tx_pool: true,
        };
        assert_eq!(attribute, &expected);

        let attribute = attributes.last().unwrap();
        let expected = ScrollPayloadAttributes{
            payload_attributes: PayloadAttributes{
                timestamp: 1696935657,
                suggested_fee_recipient: SCROLL_FEE_VAULT_ADDRESS,
                ..Default::default()
            },
            transactions: Some(vec![bytes!("f88c8202658417d7840082a4f294530000000000000000000000000000000000000280a4bede39b500000000000000000000000000000000000000000000000000000001669aa2f583104ec4a07461e6555f927393ebdf5f183738450c3842bc3b86a1db7549d9bee21fadd0b1a06d7ba96897bd9fb8e838a327d3ca34be66da11955f10d1fb2264949071e9e8cd")]),
            no_tx_pool: true,
        };
        assert_eq!(attribute, &expected);

        Ok(())
    }
}
