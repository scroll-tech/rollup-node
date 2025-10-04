use super::*;
use futures::{stream::FuturesOrdered, StreamExt};
use tokio::sync::Mutex;

/// A structure holding the current unresolved futures for the derivation pipeline.
#[derive(Debug)]
pub struct DerivationPipelineNew<P> {
    /// The active batch derivation futures.
    futures: Arc<Mutex<FuturesOrdered<DerivationPipelineFutureNew>>>,
    /// A reference to the database.
    database: Arc<Database>,
    /// A L1 provider.
    l1_provider: P,
    /// The L1 message queue index at which the V2 L1 message queue was enabled.
    l1_v2_message_queue_start_index: u64,
    /// The metrics of the pipeline.
    metrics: DerivationPipelineMetrics,
}

impl<P> DerivationPipelineNew<P> {
    /// Returns a new instance of the [`DerivationPipelineNew`].
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
        }
    }
}

impl<P> DerivationPipelineNew<P>
where
    P: L1Provider + Clone + Send + Sync + 'static,
{
    /// Pushes a new batch info to the derivation pipeline.
    pub async fn push(&mut self, batch_info: Arc<BatchInfo>) {
        let fut = self.derivation_future(batch_info);
        self.futures.lock().await.push_back(fut);
    }

    /// Returns the number of unresolved futures in the derivation pipeline.
    pub async fn len(&self) -> usize {
        self.futures.lock().await.len()
    }

    fn derivation_future(&self, batch_info: Arc<BatchInfo>) -> DerivationPipelineFutureNew {
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

impl<P> Stream for DerivationPipelineNew<P>
where
    P: L1Provider + Unpin + Clone + Send + Sync + 'static,
{
    type Item = BatchDerivationResult;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Poll the next future in the ordered set of futures.
        match this.futures.try_lock() {
            Ok(mut guard) => {
                match guard.poll_next_unpin(cx) {
                    // If the derivation failed then push it to the front of the queue to be
                    // retried.
                    Poll::Ready(Some(Err((batch_info, err)))) => {
                        tracing::error!(target: "scroll::derivation_pipeline", ?batch_info, ?err, "Failed to derive payload attributes");
                        guard.push_front(this.derivation_future(batch_info.clone()));
                        return Poll::Pending
                    }
                    // If the derivation succeeded then return the attributes.
                    Poll::Ready(Some(Ok(result))) => return Poll::Ready(Some(result)),
                    // If there are no more futures then return pending.
                    _ => return Poll::Pending,
                }
            }
            Err(_) => {
                // Could not acquire the lock, return pending.
                cx.waker().wake_by_ref();
                return Poll::Pending
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
#[derive(Debug, Clone)]
pub struct DerivedAttributes {
    /// The block number the attributes correspond to.
    pub block_number: u64,
    /// The derived payload attributes.
    pub attributes: ScrollPayloadAttributes,
}

/// A future that resolves to a stream of [`ScrollPayloadAttributesWithBatchInfo`].
type DerivationPipelineFutureNew = Pin<
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
