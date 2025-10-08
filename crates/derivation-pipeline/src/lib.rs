//! A stateless derivation pipeline for Scroll.
//!
//! This crate provides a simple implementation of a derivation pipeline that transforms a batch
//! into payload attributes for block building.

use alloy_primitives::{Address, B256};
use alloy_rpc_types_engine::PayloadAttributes;
use core::{fmt::Debug, future::Future, pin::Pin, task::Poll};
use futures::{stream::FuturesOrdered, task::AtomicWaker, Stream, StreamExt};
use rollup_node_primitives::{BatchCommitData, BatchInfo, L1MessageEnvelope};
use rollup_node_providers::{BlockDataProvider, L1Provider};
use scroll_alloy_rpc_types_engine::{BlockDataHint, ScrollPayloadAttributes};
use scroll_codec::{decoding::payload::PayloadData, Codec};
use scroll_db::{Database, DatabaseReadOperations, DatabaseTransactionProvider, L1MessageKey};
use tokio::sync::Mutex;

mod data_source;

mod error;
pub use error::DerivationPipelineError;

mod metrics;
pub use metrics::DerivationPipelineMetrics;

use crate::data_source::CodecDataSource;
use std::{boxed::Box, sync::Arc, time::Instant, vec::Vec};

/// A structure holding the current unresolved futures for the derivation pipeline.
#[derive(Debug)]
pub struct DerivationPipeline<P> {
    /// The active batch derivation futures.
    futures: Arc<Mutex<FuturesOrdered<DerivationPipelineFuture>>>,
    /// A reference to the database.
    database: Arc<Database>,
    /// A L1 provider.
    l1_provider: P,
    /// The L1 message queue index at which the V2 L1 message queue was enabled.
    l1_v2_message_queue_start_index: u64,
    /// The metrics of the pipeline.
    metrics: DerivationPipelineMetrics,
    /// The waker for the stream.
    waker: AtomicWaker,
}

impl<P> DerivationPipeline<P> {
    /// Returns a new instance of the [`DerivationPipeline`].
    pub fn new(
        l1_provider: P,
        database: Arc<Database>,
        l1_v2_message_queue_start_index: u64,
    ) -> Self {
        Self {
            futures: Arc::new(Mutex::new(FuturesOrdered::new())),
            database,
            l1_provider,
            l1_v2_message_queue_start_index,
            metrics: DerivationPipelineMetrics::default(),
            waker: AtomicWaker::new(),
        }
    }
}

impl<P> DerivationPipeline<P>
where
    P: L1Provider + Clone + Send + Sync + 'static,
{
    /// Pushes a new batch info to the derivation pipeline.
    pub async fn push_batch(&mut self, batch_info: Arc<BatchInfo>) {
        let fut = self.derivation_future(batch_info);
        self.futures.lock().await.push_back(fut);
        self.waker.wake();
    }

    /// Returns the number of unresolved futures in the derivation pipeline.
    pub async fn len(&self) -> usize {
        self.futures.lock().await.len()
    }

    /// Returns true if there are no unresolved futures in the derivation pipeline.
    pub async fn is_empty(&self) -> bool {
        self.futures.lock().await.is_empty()
    }

    fn derivation_future(&self, batch_info: Arc<BatchInfo>) -> DerivationPipelineFuture {
        let database = self.database.clone();
        let metrics = self.metrics.clone();
        let provider = self.l1_provider.clone();
        let l1_v2_message_queue_start_index = self.l1_v2_message_queue_start_index;

        Box::pin(async move {
            let derive_start = Instant::now();

            // get the batch commit data.
            let tx = database.tx().await.map_err(|e| (batch_info.clone(), e.into()))?;
            let batch = tx
                .get_batch_by_index(batch_info.index)
                .await
                .map_err(|err| (batch_info.clone(), err.into()))?
                .ok_or((
                    batch_info.clone(),
                    DerivationPipelineError::UnknownBatch(batch_info.index),
                ))?;

            // derive the attributes and attach the corresponding batch info.
            let result = derive_new(batch, provider, tx, l1_v2_message_queue_start_index)
                .await
                .map_err(|err| (batch_info.clone(), err))?;

            // update metrics.
            metrics.derived_blocks.increment(result.attributes.len() as u64);
            let execution_duration = derive_start.elapsed().as_secs_f64();
            metrics.blocks_per_second.set(result.attributes.len() as f64 / execution_duration);
            Ok(result)
        })
    }
}

impl<P> Stream for DerivationPipeline<P>
where
    P: L1Provider + Unpin + Clone + Send + Sync + 'static,
{
    type Item = BatchDerivationResult;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.waker.register(cx.waker());

        // Poll the next future in the ordered set of futures.
        match this.futures.try_lock() {
            Ok(mut guard) => {
                let result = guard.poll_next_unpin(cx);
                match result {
                    // If the derivation failed then push it to the front of the queue to be
                    // retried.
                    Poll::Ready(Some(Err((batch_info, err)))) => {
                        tracing::error!(target: "scroll::derivation_pipeline", ?batch_info, ?err, "Failed to derive payload attributes");
                        guard.push_front(this.derivation_future(batch_info));
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    // If the derivation succeeded then return the attributes.
                    Poll::Ready(Some(Ok(result))) => Poll::Ready(Some(result)),
                    // If there are no more futures then return None.
                    Poll::Ready(None) | Poll::Pending => Poll::Pending,
                }
            }
            Err(_) => {
                // Could not acquire the lock, return pending.
                cx.waker().wake_by_ref();

                Poll::Pending
            }
        }
    }
}

/// The result of deriving a batch.
#[derive(Debug)]
pub struct BatchDerivationResult {
    /// The derived payload attributes.
    pub attributes: Vec<DerivedAttributes>,
    /// The batch info associated with the derived attributes.
    pub batch_info: BatchInfo,
}

/// The derived attributes along with the block number they correspond to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedAttributes {
    /// The block number the attributes correspond to.
    pub block_number: u64,
    /// The derived payload attributes.
    pub attributes: ScrollPayloadAttributes,
}

/// A future that resolves to a stream of [`BatchDerivationResult`].
type DerivationPipelineFuture = Pin<
    Box<
        dyn Future<
                Output = Result<BatchDerivationResult, (Arc<BatchInfo>, DerivationPipelineError)>,
            > + Send,
    >,
>;

/// Returns a vector of [`ScrollPayloadAttributes`] from the [`BatchCommitData`] and a
/// [`L1Provider`].
pub async fn derive_new<L1P: L1Provider + Sync + Send, L2P: BlockDataProvider + Sync + Send>(
    batch: BatchCommitData,
    l1_provider: L1P,
    l2_provider: L2P,
    l1_v2_message_queue_start_index: u64,
) -> Result<BatchDerivationResult, DerivationPipelineError> {
    // fetch the blob then decode the input batch.
    let blob = if let Some(hash) = batch.blob_versioned_hash {
        l1_provider.blob(batch.block_timestamp, hash).await?
    } else {
        None
    };
    let data = CodecDataSource { calldata: batch.calldata.as_ref(), blob: blob.as_deref() };
    let decoded = Codec::decode(&data)?;

    // set the cursor for the l1 provider.
    let payload_data = &decoded.data;
    let mut l1_messages_iter =
        iter_l1_messages_from_payload(&l1_provider, payload_data, l1_v2_message_queue_start_index)
            .await?;

    let skipped_l1_messages = decoded.data.skipped_l1_message_bitmap.clone().unwrap_or_default();
    let mut skipped_l1_messages = skipped_l1_messages.into_iter();
    let blocks = decoded.data.into_l2_blocks();
    let mut attributes = Vec::with_capacity(blocks.len());

    for mut block in blocks {
        // query the appropriate amount of l1 messages.
        let mut txs = Vec::with_capacity(block.context.num_transactions as usize);
        for _ in 0..block.context.num_l1_messages {
            // check if the next l1 message should be skipped.
            if matches!(skipped_l1_messages.next(), Some(bit) if bit) {
                let _ = l1_messages_iter.next();
                continue;
            }

            let l1_message = l1_messages_iter
                .next()
                .ok_or(DerivationPipelineError::MissingL1Message(block.clone()))?;
            let mut bytes = Vec::with_capacity(l1_message.transaction.eip2718_encoded_length());
            l1_message.transaction.eip2718_encode(&mut bytes);
            txs.push(bytes.into());
        }

        // add the block transactions.
        txs.append(&mut block.transactions);

        // get the block data for the l2 block.
        let number = block.context.number;
        // TODO(performance): can this be improved by adding block_data_range.
        let block_data = l2_provider.block_data(number).await.map_err(Into::into)?;

        // construct the payload attributes.
        let attribute = DerivedAttributes {
            block_number: number,
            attributes: ScrollPayloadAttributes {
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
            },
        };
        attributes.push(attribute);
    }

    Ok(BatchDerivationResult {
        attributes,
        batch_info: BatchInfo { index: batch.index, hash: batch.hash },
    })
}

/// Returns an iterator over L1 messages from the `PayloadData`. If the `PayloadData` returns a
/// `prev_l1_message_queue_hash` of zero, uses the `l1_v2_message_queue_start_index` to fetch
/// messages from the L1 provider.
///
/// # Errors
///
/// Propagates any error from the L1 provider.
/// Returns an error if the retrieved number of L1 messages does not match the expected number from
/// the payload data.
async fn iter_l1_messages_from_payload<L1P: L1Provider>(
    provider: &L1P,
    data: &PayloadData,
    l1_v2_message_queue_start_index: u64,
) -> Result<Box<dyn Iterator<Item = L1MessageEnvelope> + Send>, DerivationPipelineError> {
    let total_l1_messages = data.blocks.iter().map(|b| b.context.num_l1_messages as u64).sum();

    let messages = if let Some(index) = data.queue_index_start() {
        provider
            .get_n_messages(L1MessageKey::from_queue_index(index), total_l1_messages)
            .await
            .map_err(Into::into)?
    } else if let Some(hash) = data.prev_l1_message_queue_hash() {
        // If the message queue hash is zero then we should use the V2 L1 message queue start
        // index. We must apply this branch logic because we do not have a L1
        // message associated with a queue hash of ZERO (we only compute a queue
        // hash for the first L1 message of the V2 contract).
        if hash == &B256::ZERO {
            provider
                .get_n_messages(
                    L1MessageKey::from_queue_index(l1_v2_message_queue_start_index),
                    total_l1_messages,
                )
                .await
                .map_err(Into::into)?
        } else {
            let mut messages = provider
                .get_n_messages(L1MessageKey::from_queue_hash(*hash), total_l1_messages + 1)
                .await
                .map_err(Into::into)?;
            // we skip the first l1 message, as we are interested in the one starting after
            // prev_l1_message_queue_hash.
            messages.remove(0);
            messages
        }
    } else {
        return Err(DerivationPipelineError::MissingL1MessageQueueCursor)
    };

    // Check we received the expected amount of L1 messages.
    if messages.len() as u64 != total_l1_messages {
        return Err(DerivationPipelineError::InvalidL1MessagesCount {
            expected: total_l1_messages,
            got: messages.len() as u64,
        })
    }

    Ok(Box::new(messages.into_iter()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use alloy_eips::Decodable2718;
    use alloy_primitives::{address, b256, bytes, U256};
    use futures::StreamExt;
    use rollup_node_primitives::L1MessageEnvelope;
    use rollup_node_providers::{test_utils::MockL1Provider, L1ProviderError};
    use scroll_alloy_consensus::TxL1Message;
    use scroll_alloy_rpc_types_engine::BlockDataHint;
    use scroll_codec::decoding::test_utils::read_to_bytes;
    use scroll_db::{
        test_utils::setup_test_db, DatabaseTransactionProvider, DatabaseWriteOperations,
    };
    use std::collections::HashMap;

    struct Infallible;
    impl From<Infallible> for L1ProviderError {
        fn from(_value: Infallible) -> Self {
            Self::Other("infallible")
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

    const L1_MESSAGE_INDEX_33: L1MessageEnvelope = L1MessageEnvelope {
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
    };

    const L1_MESSAGE_INDEX_34: L1MessageEnvelope = L1MessageEnvelope {
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
    };

    #[tokio::test]
    async fn test_should_retry_on_derivation_error() -> eyre::Result<()> {
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
        let tx = db.tx_mut().await?;
        tx.insert_batch(batch_data).await?;
        // load message in db, leaving a l1 message missing.
        tx.insert_l1_message(L1_MESSAGE_INDEX_33).await?;
        tx.commit().await?;

        // construct the pipeline.
        let l1_messages_provider = db.clone();
        let mock_l1_provider = MockL1Provider { l1_messages_provider, blobs: HashMap::new() };
        let mut pipeline = DerivationPipeline::new(mock_l1_provider, db.clone(), u64::MAX);

        // as long as we don't call `push_batch`, pipeline should not return attributes.
        pipeline.push_batch(BatchInfo { index: 12, hash: Default::default() }.into()).await;

        // wait for 5 seconds to ensure the pipeline is in a retry loop.
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {}
            _ = pipeline.next() => {panic!("pipeline should not yield as the transactions are not in db so it should be in a retry loop");}
        }

        // in a separate task, add the second l1 message.
        tokio::task::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            let tx = db.tx_mut().await.unwrap();
            tx.insert_l1_message(L1_MESSAGE_INDEX_34).await.unwrap();
            tx.commit().await.unwrap();
        });

        // check the correctness of the last attribute.
        let mut attribute = ScrollPayloadAttributes::default();
        if let Some(BatchDerivationResult { attributes, .. }) = pipeline.next().await {
            for a in attributes {
                if a.attributes.payload_attributes.timestamp == 1696935657 {
                    attribute = a.attributes;
                    break;
                }
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
    async fn test_should_stream_payload_attributes() -> eyre::Result<()> {
        reth_tracing::init_test_tracing();

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
        let tx = db.tx_mut().await?;
        tx.insert_batch(batch_data).await?;
        // load messages in db.
        let l1_messages = vec![L1_MESSAGE_INDEX_33, L1_MESSAGE_INDEX_34];
        for message in l1_messages {
            tx.insert_l1_message(message).await?;
        }
        tx.commit().await?;

        // construct the pipeline.
        let l1_messages_provider = db.clone();
        let mock_l1_provider = MockL1Provider { l1_messages_provider, blobs: HashMap::new() };
        let mut pipeline = DerivationPipeline::new(mock_l1_provider, db, u64::MAX);

        // as long as we don't call `push_batch`, pipeline should not return attributes.
        pipeline.push_batch(BatchInfo { index: 12, hash: Default::default() }.into()).await;

        // check the correctness of the last attribute.
        let mut attribute = ScrollPayloadAttributes::default();
        if let Some(BatchDerivationResult { attributes, .. }) = pipeline.next().await {
            for a in attributes {
                if a.attributes.payload_attributes.timestamp == 1696935657 {
                    attribute = a.attributes;
                    break
                }
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
    async fn test_should_derive_calldata_batch() -> eyre::Result<()> {
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
        let l1_messages = vec![L1_MESSAGE_INDEX_33, L1_MESSAGE_INDEX_34];
        let tx = db.tx_mut().await?;
        for message in l1_messages {
            tx.insert_l1_message(message).await?;
        }
        tx.commit().await?;

        let l1_messages_provider = db.clone();
        let l1_provider = MockL1Provider { l1_messages_provider, blobs: HashMap::new() };
        let l2_provider = MockL2Provider;

        let result = derive_new(batch_data, l1_provider, l2_provider, u64::MAX).await?;
        let attribute = result
            .attributes
            .iter()
            .find(|a| a.attributes.payload_attributes.timestamp == 1696935384)
            .unwrap();

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
        assert_eq!(attribute.attributes, expected);

        let attribute = result.attributes.last().unwrap();
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
        assert_eq!(attribute.attributes, expected);

        Ok(())
    }

    #[tokio::test]
    async fn test_should_skip_l1_messages() -> eyre::Result<()> {
        // https://sepolia.etherscan.io/tx/0xe9d7a634a2afd8adee5deab180c30d261e05fea499ccbfd5c987436fe587850e
        // load batch data in the db.
        let db = Arc::new(setup_test_db().await);
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
        let tx = db.tx_mut().await?;
        for message in l1_messages.clone() {
            tx.insert_l1_message(message).await?;
        }
        tx.commit().await?;

        let l1_messages_provider = db.clone();
        let l1_provider = MockL1Provider { l1_messages_provider, blobs: HashMap::new() };
        let l2_provider = MockL2Provider;

        // derive attributes and extract l1 messages.
        let attributes = derive_new(batch_data, l1_provider, l2_provider, u64::MAX).await?;
        let derived_l1_messages: Vec<_> = attributes
            .attributes
            .into_iter()
            .filter_map(|a| a.attributes.transactions)
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

    #[tokio::test]
    async fn test_should_skip_l1_messages_complex() -> eyre::Result<()> {
        // https://sepolia.etherscan.io/tx/0x3ac4fa531bba0cd1593e2f5e6720a6c580864665d50fbf0de4ca9d7de10c504b
        // load batch data in the db.
        let db = Arc::new(setup_test_db().await);
        let raw_calldata =
            read_to_bytes("./testdata/calldata_v0_with_skipped_l1_messages_complex.bin")?;
        let batch_data = BatchCommitData {
            hash: b256!("082A1232491ACFBB436BF37E788967773DDF3B40E0F60170355870868E45FD7F"),
            index: 38265,
            block_number: 4373247,
            block_timestamp: 1695797868,
            calldata: Arc::new(raw_calldata),
            blob_versioned_hash: None,
            finalized_block_number: None,
        };

        // prepare the l1 messages.
        let l1_messages = (777290..=777306)
            .map(|index| L1MessageEnvelope {
                l1_block_number: 0,
                l2_block_number: None,
                queue_hash: None,
                transaction: TxL1Message { queue_index: index, ..Default::default() },
            })
            .collect::<Vec<_>>();

        let tx = db.tx_mut().await?;
        for message in l1_messages.clone() {
            tx.insert_l1_message(message).await?;
        }
        tx.commit().await?;

        let l1_messages_provider = db.clone();
        let l1_provider = MockL1Provider { l1_messages_provider, blobs: HashMap::new() };
        let l2_provider = MockL2Provider;

        // derive attributes and extract l1 messages.
        let attributes = derive_new(batch_data, l1_provider, l2_provider, u64::MAX).await?;
        let derived_l1_messages: Vec<_> = attributes
            .attributes
            .into_iter()
            .filter_map(|a| a.attributes.transactions)
            .flatten()
            .filter_map(|rlp| {
                let buf = &mut rlp.as_ref();
                TxL1Message::decode_2718(buf).ok()
            })
            .collect();

        // skipped bitmap should be [0, 1, 0, 1, 0, 1, 0, 1], [0, 1, 0, 1, 0, 1, 0, 1], [0..0], ..
        // meaning every second message is skipped.
        let expected_l1_messages: Vec<_> = l1_messages
            .into_iter()
            .enumerate()
            .filter_map(|(index, message)| (index % 2 == 0).then_some(message.transaction))
            .collect();
        assert_eq!(expected_l1_messages, derived_l1_messages);
        Ok(())
    }

    #[test]
    #[allow(clippy::large_stack_frames)]
    fn test_should_derive_blob_batch() -> eyre::Result<()> {
        let handle = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
                rt.block_on(async {
                    // <https://etherscan.io/tx/0xee0afe29207fe23626387bc8eb209ab751c1fee9c18e3d6ec7a5edbcb5a4fed4>
                    // load batch data in the db.
                    let db = Arc::new(setup_test_db().await);
                    let commit_calldata = read_to_bytes("./testdata/calldata_v4_compressed.bin")?;
                    let blob = read_to_bytes("./testdata/blob_v4_compressed.bin")?;
                    let batch_data = BatchCommitData {
                        hash: b256!("fdd4ed0eb20398b3fc490ec976dd2ed99f1a898540a18874f302b38732e57431"),
                        index: 314189,
                        block_number: 20677405,
                        block_timestamp: 1725455135,
                        calldata: Arc::new(commit_calldata),
                        blob_versioned_hash: Some(b256!(
                            "013b3960a40175bd6436e8dfe07e6d80c125e12997fa1de004b1990e20dba1ee"
                        )),
                        finalized_block_number: None,
                    };
                    let l1_messages = vec![
                        L1MessageEnvelope {
                            l1_block_number: 0,
                            l2_block_number: None,
                            queue_hash: None,
                            transaction: TxL1Message {
                                queue_index: 932910,
                                gas_limit: 168000,
                                to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
                                value: U256::ZERO,
                                sender: address!("7885bcbd5cecef1336b5300fb5186a12ddd8c478"),
                                input: bytes!("8ef1332e0000000000000000000000001812f0e31dfc99c1f64ef69767a80424c299579c0000000000000000000000001812f0e31dfc99c1f64ef69767a80424c299579c00000000000000000000000000000000000000000000000038233cd84fbc3b9c00000000000000000000000000000000000000000000000000000000000e3c2e00000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000"),
                            },
                        },
                        L1MessageEnvelope {
                            l1_block_number: 0,
                            l2_block_number: None,
                            queue_hash: None,
                            transaction: TxL1Message {
                                queue_index: 932911,
                                gas_limit: 168000,
                                to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
                                value: U256::ZERO,
                                sender: address!("7885bcbd5cecef1336b5300fb5186a12ddd8c478"),
                                input: bytes!("8ef1332e000000000000000000000000e8c11b95621c80ac03f41bc33b36f343a1d95a25000000000000000000000000e8c11b95621c80ac03f41bc33b36f343a1d95a2500000000000000000000000000000000000000000000000002c68af0bb14000000000000000000000000000000000000000000000000000000000000000e3c2f00000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000"),
                            },
                        },
                        L1MessageEnvelope {
                            l1_block_number: 0,
                            l2_block_number: None,
                            queue_hash: None,
                            transaction: TxL1Message {
                                queue_index: 932912,
                                gas_limit: 168000,
                                to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
                                value: U256::ZERO,
                                sender: address!("7885bcbd5cecef1336b5300fb5186a12ddd8c478"),
                                input: bytes!("8ef1332e000000000000000000000000de9692389a2883b0e74070d6f17fbb4d32741e68000000000000000000000000de9692389a2883b0e74070d6f17fbb4d32741e68000000000000000000000000000000000000000000000000068954012935800000000000000000000000000000000000000000000000000000000000000e3c3000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000"),
                            },
                        },
                        L1MessageEnvelope {
                            l1_block_number: 0,
                            l2_block_number: None,
                            queue_hash: None,
                            transaction: TxL1Message {
                                queue_index: 932913,
                                gas_limit: 168000,
                                to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
                                value: U256::ZERO,
                                sender: address!("7885bcbd5cecef1336b5300fb5186a12ddd8c478"),
                                input: bytes!("8ef1332e00000000000000000000000095a8fe010ec6f0ca854dd78c46b9c4cbedac117900000000000000000000000095a8fe010ec6f0ca854dd78c46b9c4cbedac1179000000000000000000000000000000000000000000000000063eb89da4ed000000000000000000000000000000000000000000000000000000000000000e3c3100000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000"),
                            },
                        },
                    ];
                    let tx = db.tx_mut().await?;
                    for message in l1_messages {
                        tx.insert_l1_message(message).await?;
                    }
                    tx.commit().await?;

                    let l1_messages_provider = db.clone();
                    let l1_provider = MockL1Provider {
                        l1_messages_provider,
                        blobs: HashMap::from([(
                            batch_data.blob_versioned_hash.unwrap(),
                            blob.to_vec().as_slice().try_into()?,
                        )]),
                    };
                    let l2_provider = MockL2Provider;

                    let attributes = derive_new(batch_data, l1_provider, l2_provider, u64::MAX).await?;

                    let attribute = attributes.attributes.last().unwrap();
                    let expected = ScrollPayloadAttributes {
                        payload_attributes: PayloadAttributes {
                            timestamp: 1725455077,
                            ..Default::default()
                        },
                        transactions: Some(vec![bytes!("02f9017a830827501d8402c15db28404220c8b833bf0fa94d6238ad2887166031567616d9a54b21eb70e4dfd865af3107a4000b901042f73d60a000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000005af3107a400000000000000000000000000000000000000000000000000000000000000000640000000000000000000000000000000000000000000000000000000000000005727465727400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000045254525400000000000000000000000000000000000000000000000000000000c080a01ab3cf2a93857170eb1a8a564a00dc54d9dbc081aff236614c05f00f89564e7ea076143846b8e83dbbedc9f7f39d9e1efafd2aa323af5977acbc3b7559eaa61338"), bytes!("02f90213830827505d830ebf5b8403c6fdd68303160094aaaaaaaacb71bf2c8cae522ea5fa455571a7410680b901a4a15112f900000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000014000000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a4000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f9700000000000000000000000000000000000000000000000000000000000001a40000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001a96557e8b05a2e0dd00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010001000000000000000000000000000000000000000000000000000000001ce571940000000000000000000000000000000000000000000000000000000000000000c001a038385859bdc661006ee04173ef0c5e7d259f213b38ec65c5ac5664cc2263588aa06edfcce7499e39f78ff336265222272f75e3b8b6292bc5e7a9b785ec2764357f"), bytes!("f901d43c84039387008301eb0694dc3d8318fbaec2de49281843f5bba22e78338146870110d9316ec000b901647c2ccc45000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000006d1aa44dfe55c66e2dd413b045aaf3db92e8bf920000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004108690eca0b490e1a9ebaf85710cce8dd72d48eeb6e74f03fcf1ea58638afbe8808c5076a512e0993e77b117e938d8505bd41380209374a5fa1736040386f9c7a1c0000000000000000000000000000000000000000000000000000000000000083104ec3a00a9daf43e323158d459652563edb141a0df3f2b6d890f6307aac52c74e0bbbbfa02163e491f0cfbeec828ccf98b8877db9ba2a6552e18b2d4d3a8c5ded1d407d73"), bytes!("f8948201ef8402faf08083018a31940241fb446d6793866245b936f2c3418f818bdcd3879970b65dfdc000a4b6b55f250000000000000000000000000000000000000000000000000098c445ad57800083104ec4a097f352f786ffb1ddf9d942286cbd9ff6839f46093767c4326a1cf9bc1f117500a048cfccfe406c692cc717a670a0148fef79789aceb282cc3d0e6805593ad605cf"), bytes!("f90bd45c8402faf080830e3b1994a2a9fd768d482caf519d749d3123a133db278a66876a94d74f42ffffb90b645973bd5e000000000000000000000000eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f97000000000000000000000000000000000000000000000000006a94d74f42ffff000000000000000000000000000000000000000000000003e3fdab75eefcae8200000000000000000000000083412753e54768f8bed921e5556680e7a3e1910800000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000066d85f8c0000000000000000000000000000000000000000000000000000000000000a600000000000000000000000006131b5fae19ea4f9d964eac0408e4408b66337b5000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000009e4e21fd0e90000000000000000000000000000000000000000000000000000000000000020000000000000000000000000f40442e1cb0bdfb496e8b7405d0c1c48a81bc897000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000004e000000000000000000000000000000000000000000000000000000000000007600000000000000000000000000000000000000000000000000000000000000420000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c00000000000000000000000005300000000000000000000000000000000000004000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f970000000000000000000000006131b5fae19ea4f9d964eac0408e4408b66337b50000000000000000000000000000000000000000000000000000000066d85f8c00000000000000000000000000000000000000000000000000000000000003c00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000000401b96cfd40000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000c00000000000000000000000008f8ed95b3b3ed2979d1ee528f38ca3e481a94dd9000000000000000000000000530000000000000000000000000000000000000400000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a4000000000000000000000000f40442e1cb0bdfb496e8b7405d0c1c48a81bc897000000000000000000000000000000000000000000000000006a94d74f42ffff0000000000000000000000000000000000000000000000000000000000030f0b000000000000000000000000000000000000000000000000000000000000004063407a490000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000f40442e1cb0bdfb496e8b7405d0c1c48a81bc897000000000000000000000000ccdf79ced5fd02af299d3548b4e35ed6163064bf00000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a4000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f9700000000000000000000000000000000000000000000000000000000044a4e800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000041783c314dc20000000000000003e6fce47750c3ecea0000000000000000000000005300000000000000000000000000000000000004000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f97000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000001a000000000000000000000000000000000000000000000000000000000000001e00000000000000000000000000000000000000000000000000000000000000220000000000000000000000000e7a23e2f9abf813ad55e55ce26c0712bf1593332000000000000000000000000000000000000000000000000006a94d74f42ffff000000000000000000000000000000000000000000000003da07eeddb6d6509a00000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000002600000000000000000000000000000000000000000000000000000000000000001000000000000000000000000f40442e1cb0bdfb496e8b7405d0c1c48a81bc8970000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000006a94d74f42ffff0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000a8337cce66f217701071a68a503caa8bf139b1840000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000001e000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002317b22536f75726365223a22686970706f2d73776170222c22416d6f756e74496e555344223a2237322e323133343430313133383636222c22416d6f756e744f7574555344223a2237312e3931343239373937333238373232222c22526566657272616c223a22222c22466c616773223a302c22416d6f756e744f7574223a223731373638373037373539383535313532373730222c2254696d657374616d70223a313732353435353036382c22496e74656772697479496e666f223a7b224b65794944223a2231222c225369676e6174757265223a22436d38564441314d3466386b696f525546383976695344495765474e785858726c3842643651396a70493057764c652b6f4d655246394472484e73544c463045315a5842736e55384b4a393173693447674631524d74614d334c777030324f5136383879704e796e436b3978425a4b2b796b427074416647614b35516a49794f45712f36494d4a654d3772626f59444675713166414f7370394634683543714c44622f7469722b507562677131474b693742556a6d6433584463796239386a70377a5132783533744e52766e52683955484f44636932516252634a6e337272394157327252397653697a4b46336874676c546b794a6a61725251446e735644772f3274687447595a4e7a516a417361354b717236323679796e466f49493175387779714b547a6b3052512f6f464d4a7a55752b454563704d752b4c626a7835322f50556c6735526678666568507066666a4a53514a773d3d227d7d0000000000000000000000000000000000000000000000000000000000000000000000000000000000000083104ec3a0aa2e2380709f2b0c6b8fcc48ba4c6942aea501a867d2bfe27a5979a9900b9692a044a21df53a177fff5c1348b3cdb23f82bab41b8fea58d68138234a302d87d904"), bytes!("f88d8201078403938700830100a794e6feca764b7548127672c189d303eb956c3ba37280a4e95a644f000000000000000000000000000000000000000000000000000000000134da0883104ec3a054336f213352ee4faf92a752befcfe39c7a6a18ce3d7bcc56b6dc875454d76dba00e9ebd78c3aa468f4d311e3ebfeb0c8c79a1f3cfc3a26ed71d3665fc6820b5d5"), bytes!("02f8b28308275024830ebf5b8403c6fdd6830415a894ec53c830f4444a8a56455c6836b5d2aa794289aa80b844830cbbbd000000000000000000000000274c3795dadfebf562932992bf241ae087e0a98c00000000000000000000000000000000000000000000000021b745fecb550714c080a0e8f444aca5c459c27676e185579de1d6cb5eb88d4e350fd6dade07946ede16a8a0165e301acebf43a608e747e07b2e32ddd86cebb24ec98440c34b919c49fcedd7"), bytes!("02f9025483082750518403db832c8403db832c8308bcf394c47300428b6ad2c7d03bb76d05a176058b47e6b080b901e4f17325e70000000000000000000000000000000000000000000000000000000000000020d57de4f41c3d3cc855eadef68f98c0d4edd22d57161d96b7c06d2f4336cc3b490000000000000000000000000000000000000000000000000000000000000040000000000000000000000000295f5db3e40c5155271eaf9058d54b185c5fff1300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000002dbce60ebeaafb77e5472308f432f78ac3ae07d90000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000004000000000000000000000000074670a3998d9d6622e32d0847ff5977c37e0ec91000000000000000000000000000000000000000000000000000000000004e900c080a0f63bb59e762a76fe9252065519362897d63795bedffe88fc922a683c82e7e8d0a07693bc90d37b2922650f8e065a0daac6ea71af62c83b4fa69285e7c511785b7a"), bytes!("f902ae819a8403ef1480830ae583940b4d5229bb5201e277d3937ce1704227c96bbc5f80b902443c0427150000000000000000000000000000000000000000000000000000000000000020d57de4f41c3d3cc855eadef68f98c0d4edd22d57161d96b7c06d2f4336cc3b4900000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000001bd63bc394b1e44a60f0d5ea4fbb61937d973ddcd80b241370f7939607494853112eb2c36e5e4a5d7b9184961380642680541125dfdd9c0764b7a2efad85f926c30000000000000000000000001f4a828ff025fa8270bfd1d4d952e75079bb593d0000000000000000000000000000000000000000000000000000000066d868f00000000000000000000000005b0d7cfaf6557f026591fc29b8f050d7537b476400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000c5d859d4bb0963c8f946d3b3751e4976165b38e0000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000000083104ec3a088eb2685bd6b79ae008d6b0ee74b09341d79f590c8b3faea5dfa2237644bc716a01482d3425fa581941b057a4d20960a90f20ce9e43fb425e537d565be705b3e10"), bytes!("02f8768308275083018b348398968085012d5fe6308275309409dcae886c35e45f2545c0087725e36e18b032eb865af3107a43ea80c001a0922e3023fc0a04bb29ec74efecebb381535ef2453907b101b342f8254fa73072a04b5c97606536ab8d9b7ffec96fab415729c53d18b5ff12f5a4fd148f4aa42d15"), bytes!("02f8b1830827505e830ebf5b8403c6fdd682d9c494e97c507e2b88ab55c61d528f506e13e35dcb8f1580b844a22cb4650000000000000000000000000cab6977a9c70e04458b740476b498b2140196410000000000000000000000000000000000000000000000000000000000000001c080a0dab66684749d0773893d7cce976dc4a8d2182db8a59044cd9bbd4d0d64f432a1a01bfe61ca3ceae4a81c3668369c36a0eda672f45d63e3935b13f2a694611da6d6"), bytes!("02f902138308275059830ebf5b8403c6fdd68306cff794c47300428b6ad2c7d03bb76d05a176058b47e6b080b901a4f17325e70000000000000000000000000000000000000000000000000000000000000020d57de4f41c3d3cc855eadef68f98c0d4edd22d57161d96b7c06d2f4336cc3b490000000000000000000000000000000000000000000000000000000000000040000000000000000000000000dcde5e9d35d5a1fe9e0eb3185459b3323e09b73b00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000001121f46b5581b5285bc571703cd772b336aa12e600000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000000c001a054dc43a6d710d99764add84512d022b4961dd39bfe366967acec0d39a5ab820fa050362c7b199a5bae81b1b56b2bb70058f64a70a01044821232b2c11aef78250d"), bytes!("02f8b983082750288402e577518402f49919830309f194ec53c830f4444a8a56455c6836b5d2aa794289aa86d8bcb85faa78b844f2b9fdb8000000000000000000000000274c3795dadfebf562932992bf241ae087e0a98c0000000000000000000000000000000000000000000000000000d8bcb85faa78c080a0b12f8b14c62254ac38a2e0f567fa67c642ed485c43d9f2db2a1f93ada8173c86a0178e3a021c50b57fc6f949201d2e1667a243ba0ea94dbf3199abeecea6c20194"), bytes!("f88c81a78402faf080830100a794e6feca764b7548127672c189d303eb956c3ba37280a4e95a644f000000000000000000000000000000000000000000000000000000000134da0883104ec3a059393dff7dc95d7e2f74053268bc1ab67e03ea0aa115929cd31912fca57d8c66a07330a31a06c957e6b4b31eee08a37c5dc15e31d1a37d4284475adc0a248fa920"),]),
                        no_tx_pool: true,
                        block_data_hint: BlockDataHint::none(),
                        gas_limit: Some(10_000_000),
                    };
                    assert_eq!(attribute.attributes, expected);

                    Result::<(), eyre::Report>::Ok(())
                })
            })?;

        handle.join().unwrap()?;

        Ok(())
    }
}
