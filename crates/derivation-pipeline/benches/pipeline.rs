//! Benchmarks for the derivation pipeline.

#![allow(missing_docs)]

use alloy_primitives::{Bytes, B256};
use criterion::{criterion_group, criterion_main, Criterion};
use futures::StreamExt;
use rollup_node_primitives::{BatchCommitData, BatchInfo, BatchStatus, L1MessageEnvelope};
use rollup_node_providers::{
    test_utils::MockL1Provider, FullL1Provider, L1Provider, S3BlobProvider,
};
use scroll_alloy_consensus::TxL1Message;
use scroll_db::{
    test_utils::setup_test_db, Database, DatabaseConnectionProvider, DatabaseWriteOperations,
    EntityTrait,
};
use scroll_derivation_pipeline::DerivationPipeline;
use std::{collections::HashMap, future::Future, path::PathBuf, pin::Pin, sync::Arc};
use tokio::runtime::{Handle, Runtime};

const BATCHES_START_INDEX: u64 = 414261;
const BATCHES_STOP_INDEX: u64 = 414513;

/// Set up a mock provider instance.
fn setup_mock_provider(
    db: Arc<Database>,
) -> Pin<Box<dyn Future<Output = MockL1Provider<Arc<Database>>> + Send>> {
    Box::pin(async {
        let mut blobs = HashMap::new();
        let db_inner = db.inner();
        let mut batches = scroll_db::batch_commit::Entity::find()
            .stream(db_inner.get_connection())
            .await
            .expect("failed to get batches stream");

        while let Some(Ok(batch)) = batches.next().await {
            let batch = Into::<BatchCommitData>::into(batch);
            if let Some(blob_hash) = batch.blob_versioned_hash {
                blobs.insert(
                    blob_hash,
                    PathBuf::from(format!("./benches/testdata/blob/blob_{}.bin", batch.index)),
                );
            }
        }

        MockL1Provider { db, blobs }
    })
}

/// Set up a full provider instance
fn setup_full_provider(
    db: Arc<Database>,
) -> Pin<Box<dyn Future<Output = FullL1Provider<Arc<Database>, S3BlobProvider>> + Send>> {
    Box::pin(async {
        let blob_provider = S3BlobProvider::new_http(
            reqwest::Url::parse("https://scroll-mainnet-blob-data.s3.us-west-2.amazonaws.com")
                .unwrap(),
        );
        FullL1Provider::new(blob_provider, db).await
    })
}

/// The L1 provider factory function.
type L1ProviderFactory<P> =
    Box<dyn Fn(Arc<Database>) -> Pin<Box<dyn Future<Output = P> + Send>> + Send>;

/// Returns a pipeline with a provider initiated from the factory function.
async fn setup_pipeline<P: L1Provider + Clone + Send + Sync + 'static>(
    factory: L1ProviderFactory<P>,
) -> (DerivationPipeline, Vec<BatchCommitData>) {
    // load batch data in the db.
    let db = Arc::new(setup_test_db().await);
    let blob_hashes: Vec<B256> = serde_json::from_str(
        &std::fs::read_to_string("./benches/testdata/batch_info.json").unwrap(),
    )
    .unwrap();
    let mut batches = vec![];

    for (index, hash) in (BATCHES_START_INDEX..=BATCHES_STOP_INDEX).zip(blob_hashes.into_iter()) {
        let raw_calldata =
            std::fs::read(format!("./benches/testdata/calldata/calldata_batch_{index}.bin"))
                .unwrap();
        let batch_data = BatchCommitData {
            hash: B256::random(),
            index,
            block_number: 18319648 + index,
            block_timestamp: 1696935971 + index,
            calldata: Arc::new(raw_calldata.into()),
            blob_versioned_hash: Some(hash),
            finalized_block_number: None,
            reverted_block_number: None,
        };
        db.insert_batch(batch_data.clone()).await.unwrap();
        batches.push(batch_data);
    }

    // load messages in db.
    let l1_messages: Vec<_> = serde_json::from_str::<Vec<(Bytes, B256)>>(
        &std::fs::read_to_string("./benches/testdata/l1_messages.json").unwrap(),
    )
    .unwrap()
    .into_iter()
    .map(|(bytes, queue_hash)| (TxL1Message::rlp_decode(&mut bytes.as_ref()).unwrap(), queue_hash))
    .map(|(tx, queue_hash)| L1MessageEnvelope::new(tx, 0, None, Some(queue_hash)))
    .collect();

    for message in l1_messages {
        db.insert_l1_message(message).await.unwrap();
    }

    // construct the pipeline.
    let l1_provider = factory(db.clone()).await;
    (DerivationPipeline::new(l1_provider, db, u64::MAX).await, batches)
}

/// Benchmark the derivation pipeline with blobs fetched from file. This does not bench the network
/// call to the AWS S3 blob storage.
fn benchmark_pipeline_derivation_in_file_blobs(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.bench_function("pipeline_derive_in_file_blobs", |b| {
        b.to_async(&rt).iter_batched(
            || {
                let (tx, rx) = std::sync::mpsc::channel();
                Handle::current().spawn(async move {
                    // setup (not measured): create fresh pipeline with 253 committed batches
                    let (mut pipeline, batches) =
                        setup_pipeline(Box::new(setup_mock_provider)).await;

                    // commit 253 batches.
                    for batch in batches {
                        let batch_info = BatchInfo { index: batch.index, hash: batch.hash };
                        pipeline.push_batch(batch_info, BatchStatus::Committed).await;
                    }

                    tx.send(pipeline).unwrap();
                });
                rx.recv().unwrap()
            },
            |mut pipeline| async move {
                // measured work.
                for _ in BATCHES_START_INDEX..=BATCHES_STOP_INDEX {
                    let _ = pipeline.next().await;
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

/// Benchmark the derivation pipeline with blobs fetched from S3.
fn benchmark_pipeline_derivation_s3_blobs(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("pipeline_derive_s3_blobs");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(20));

    group.bench_function("pipeline_derive_s3_blobs", |b| {
        b.to_async(&rt).iter_batched(
            || {
                let (tx, rx) = std::sync::mpsc::channel();
                Handle::current().spawn(async move {
                    // setup (not measured): create fresh pipeline with 15 committed batches
                    let (mut pipeline, batches) =
                        setup_pipeline(Box::new(setup_full_provider)).await;

                    // commit 15 batches.
                    for batch in batches {
                        let batch_info = BatchInfo { index: batch.index, hash: batch.hash };
                        pipeline.push_batch(batch_info, BatchStatus::Committed).await;
                    }

                    tx.send(pipeline).unwrap();
                });
                rx.recv().unwrap()
            },
            |mut pipeline| async move {
                // measured work.
                for _ in BATCHES_START_INDEX..=BATCHES_START_INDEX + 15 {
                    let _ = pipeline.next().await;
                }
            },
            criterion::BatchSize::LargeInput,
        )
    });
}

criterion_group!(
    benches,
    benchmark_pipeline_derivation_in_file_blobs,
    benchmark_pipeline_derivation_s3_blobs
);
criterion_main!(benches);
