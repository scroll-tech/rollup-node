//! Benchmarks for the derivation pipeline.

#![allow(missing_docs)]

use alloy_primitives::{Bytes, B256};
use criterion::{criterion_group, criterion_main, Criterion};
use futures::StreamExt;
use reth_tracing::init_test_tracing;
use rollup_node_primitives::{BatchCommitData, BatchInfo, L1MessageEnvelope};
use rollup_node_providers::{
    test_utils::MockL1Provider, FullL1Provider, L1Provider, S3BlobProvider,
};
use scroll_alloy_consensus::TxL1Message;
use scroll_db::{
    test_utils::setup_test_db, Database, DatabaseReadOperations, DatabaseTransactionProvider,
    DatabaseWriteOperations,
};
use scroll_derivation_pipeline::DerivationPipeline;
use std::{collections::HashMap, future::Future, path::PathBuf, pin::Pin, sync::Arc};
use tokio::runtime::{Handle, Runtime};

fn setup_mock_provider(
    db: Arc<Database>,
) -> Pin<Box<dyn Future<Output = MockL1Provider<Arc<Database>>> + Send>> {
    Box::pin(async {
        let tx = db.tx().await.expect("failed to get tx");
        let mut blobs = HashMap::new();
        let mut batches = tx.get_batches().await.expect("failed to get batches stream");

        while let Some(Ok(batch)) = batches.next().await {
            if let Some(blob_hash) = batch.blob_versioned_hash {
                blobs.insert(
                    blob_hash,
                    PathBuf::from(format!("./benches/testdata/blob/blob_{}.bin", batch.index)),
                );
            }
        }

        MockL1Provider { l1_messages_provider: db, blobs }
    })
}

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

async fn setup_pipeline<P: L1Provider + Clone + Send + Sync + 'static>(
    setup: Box<dyn Fn(Arc<Database>) -> Pin<Box<dyn Future<Output = P> + Send>> + Send>,
) -> DerivationPipeline<P> {
    // load batch data in the db.
    let db = Arc::new(setup_test_db().await);
    let blob_hashes: Vec<B256> = serde_json::from_str(
        &std::fs::read_to_string("./benches/testdata/batch_info.json").unwrap(),
    )
    .unwrap();

    let tx = db.tx_mut().await.unwrap();
    for (index, hash) in (414261..=414513).zip(blob_hashes.into_iter()) {
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
        };
        tx.insert_batch(batch_data).await.unwrap();
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
        tx.insert_l1_message(message).await.unwrap();
    }
    tx.commit().await.unwrap();

    // construct the pipeline.
    let l1_provider = setup(db.clone()).await;
    DerivationPipeline::new(l1_provider, db, u64::MAX)
}

fn benchmark_pipeline_derivation_in_file_blobs(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let start_index = 414261;
    let end_index = 414513;

    c.bench_function("pipeline_derive_in_file_blobs", |b| {
        b.to_async(&rt).iter_batched(
            || {
                let (tx, rx) = std::sync::mpsc::channel();
                Handle::current().spawn(async move {
                    // setup (not measured): create fresh pipeline with 253 committed batches
                    let mut pipeline = setup_pipeline(Box::new(setup_mock_provider)).await;

                    // commit 253 batches.
                    for index in start_index..=end_index {
                        let batch_info = BatchInfo { index, hash: Default::default() };
                        pipeline.push_batch(batch_info, 0);
                    }

                    tx.send(pipeline).unwrap();
                });
                rx.recv().unwrap()
            },
            |mut pipeline| async move {
                // measured work.
                for _ in start_index..=end_index {
                    let _ = pipeline.next().await;
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

fn benchmark_pipeline_derivation_s3_blobs(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    // Bench 15 batches.
    let start_index = 414261;
    let end_index = 414276;
    init_test_tracing();
    let mut group = c.benchmark_group("pipeline_derive_s3_blobs");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(20));

    group.bench_function("pipeline_derive_s3_blobs", |b| {
        b.to_async(&rt).iter_batched(
            || {
                let (tx, rx) = std::sync::mpsc::channel();
                Handle::current().spawn(async move {
                    // setup (not measured): create fresh pipeline with 15 committed batches
                    let mut pipeline = setup_pipeline(Box::new(setup_full_provider)).await;

                    // commit 15 batches.
                    for index in start_index..=end_index {
                        let batch_info = BatchInfo { index, hash: Default::default() };
                        pipeline.push_batch(batch_info, 0);
                    }

                    tx.send(pipeline).unwrap();
                });
                rx.recv().unwrap()
            },
            |mut pipeline| async move {
                // measured work.
                for _ in start_index..=end_index {
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
