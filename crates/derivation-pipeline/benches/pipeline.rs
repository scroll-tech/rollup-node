//! Benchmarks for the derivation pipeline.

#![allow(missing_docs)]

use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{address, b256, bytes, U256};
use criterion::{criterion_group, criterion_main, Criterion};
use futures::StreamExt;
use rollup_node_primitives::{BatchCommitData, BatchInfo, L1MessageEnvelope};
use rollup_node_providers::{test_utils::MockL1Provider, DatabaseL1MessageProvider};
use scroll_alloy_consensus::TxL1Message;
use scroll_codec::decoding::test_utils::read_to_bytes;
use scroll_db::{
    test_utils::setup_test_db, Database, DatabaseTransactionProvider, DatabaseWriteOperations,
};
use scroll_derivation_pipeline::DerivationPipeline;
use tokio::runtime::{Handle, Runtime};

async fn setup_pipeline(
) -> DerivationPipeline<MockL1Provider<DatabaseL1MessageProvider<Arc<Database>>>> {
    // load batch data in the db.
    let db = Arc::new(setup_test_db().await);
    let raw_calldata = read_to_bytes("./testdata/calldata_v0.bin").unwrap();
    let batch_data = BatchCommitData {
        hash: b256!("7f26edf8e3decbc1620b4d2ba5f010a6bdd10d6bb16430c4f458134e36ab3961"),
        index: 12,
        block_number: 18319648,
        block_timestamp: 1696935971,
        calldata: Arc::new(raw_calldata),
        blob_versioned_hash: None,
        finalized_block_number: None,
    };
    let tx = db.tx_mut().await.unwrap();
    tx.insert_batch(batch_data).await.unwrap();

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
        tx.insert_l1_message(message).await.unwrap();
    }
    tx.commit().await.unwrap();

    // construct the pipeline.
    let l1_messages_provider = DatabaseL1MessageProvider::new(db.clone(), 0);
    let mock_l1_provider = MockL1Provider { l1_messages_provider, blobs: HashMap::new() };
    DerivationPipeline::new(mock_l1_provider, db, u64::MAX)
}

fn benchmark_pipeline_derivation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.bench_function("pipeline_derive_1000_batches", |b| {
        b.to_async(&rt).iter_batched(
            || {
                let (tx, rx) = std::sync::mpsc::channel();
                Handle::current().spawn(async move {
                    // setup (not measured): create fresh pipeline with 1000 committed batches
                    let mut pipeline = setup_pipeline().await;
                    let batch_info = BatchInfo { index: 12, hash: Default::default() };

                    // commit 1000 batches.
                    for _ in 0..1000 {
                        pipeline.push_batch(batch_info, 0);
                    }

                    tx.send(pipeline).unwrap();
                });
                rx.recv().unwrap()
            },
            |mut pipeline| async move {
                // measured work: derive 1000 batches.
                for _ in 0..1000 {
                    let _ = pipeline.next().await.unwrap();
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, benchmark_pipeline_derivation);
criterion_main!(benches);
