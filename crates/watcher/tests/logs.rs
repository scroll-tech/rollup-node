//! Integration test of the logs for the L1 watcher.
#![cfg(feature = "test-utils")]

use alloy_rpc_types_eth::Log;
use alloy_sol_types::SolEvent;
use arbitrary::Arbitrary;
use rollup_node_primitives::{L1BlockStartupInfo, NodeConfig};
use rollup_node_watcher::{
    random,
    test_utils::{chain, chain_from, provider::MockProvider},
    Block, L1Notification, L1Watcher,
};
use scroll_alloy_consensus::TxL1Message;
use scroll_l1::abi::logs::{try_decode_log, QueueTransaction};
use std::sync::Arc;

#[tokio::test]
async fn test_should_not_miss_logs_on_reorg() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    const CHAIN_LEN: usize = 200;
    const HALF_CHAIN_LEN: usize = CHAIN_LEN / 2;
    const LOGS_QUERY_BLOCK_RANGE: u64 = 500;

    // Given
    let (finalized, _, headers) = chain(CHAIN_LEN);
    let reorged = chain_from(&headers[HALF_CHAIN_LEN], HALF_CHAIN_LEN);

    let finalized_blocks = std::iter::repeat_n(finalized, 2 * CHAIN_LEN)
        .map(|h| Block { header: h, ..Default::default() })
        .collect();
    let latest_blocks: Vec<_> = [headers, reorged]
        .concat()
        .into_iter()
        .map(|h| Block { header: h, ..Default::default() })
        .collect();
    let logs: Vec<_> = latest_blocks
        .clone()
        .into_iter()
        .skip(1)
        .map(|b| {
            let mut queue_transaction = random!(Log);
            let mut inner_log = random!(alloy_primitives::Log);
            inner_log.data = random!(QueueTransaction).encode_log_data();
            queue_transaction.inner = inner_log;
            queue_transaction.block_number = Some(b.header.number);
            queue_transaction.block_timestamp = Some(b.header.timestamp);
            queue_transaction.block_hash = Some(b.header.hash);
            queue_transaction
        })
        .collect();
    let last_log = logs.last().unwrap().clone();

    let config = NodeConfig {
        start_l1_block: latest_blocks.first().unwrap().header.number,
        address_book: Default::default(),
    };
    let mock_provider = MockProvider::new(
        latest_blocks.clone().into_iter(),
        std::iter::empty(),
        logs.clone().into_iter(),
        finalized_blocks,
        latest_blocks,
    );

    // spawn the watcher and verify received notifications are consistent.
    let (_, mut l1_watcher) = L1Watcher::spawn(
        mock_provider,
        L1BlockStartupInfo::None,
        Arc::new(config),
        LOGS_QUERY_BLOCK_RANGE,
        false,
    )
    .await;
    let mut received_logs = Vec::new();
    loop {
        let notification =
            l1_watcher.l1_notification_receiver().recv().await.map(|notif| (*notif).clone());
        if let Some(L1Notification::L1Message { block_timestamp, message, .. }) = notification {
            received_logs.push(message);
            if block_timestamp == last_log.block_timestamp.unwrap() {
                break
            }
        }
    }

    let expected_logs: Vec<_> = logs
        .into_iter()
        .filter_map(|l| {
            try_decode_log::<QueueTransaction>(&l.inner)
                .map(|log| Into::<TxL1Message>::into(log.data))
        })
        .collect();
    assert_eq!(expected_logs, received_logs);

    Ok(())
}
