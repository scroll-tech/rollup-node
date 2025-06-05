//! Integration test of the logs for the L1 watcher.
#![cfg(feature = "test-utils")]

use alloy_rpc_types_eth::Log;
use alloy_sol_types::SolEvent;
use arbitrary::Arbitrary;
use rollup_node_primitives::NodeConfig;
use rollup_node_watcher::{
    random,
    test_utils::{chain, provider::MockProvider},
    Block, L1Notification, L1Watcher,
};
use scroll_l1::abi::logs::QueueTransaction;
use std::sync::Arc;
use tokio::select;

#[tokio::test]
async fn test_should_not_index_latest_block_multiple_times() -> eyre::Result<()> {
    const CHAIN_LEN: usize = 200;
    const HALF_CHAIN_LEN: usize = 100;

    // Given
    let (finalized, latest, headers) = chain(CHAIN_LEN);

    let finalized_blocks = std::iter::repeat_n(finalized, 2 * CHAIN_LEN)
        .map(|h| Block { header: h, ..Default::default() })
        .collect();
    let latest_blocks: Vec<_> = headers
        .into_iter()
        .chain(std::iter::repeat_n(latest, HALF_CHAIN_LEN))
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
            queue_transaction
        })
        .collect();

    let config = NodeConfig {
        start_l1_block: latest_blocks.first().unwrap().header.number,
        address_book: Default::default(),
    };
    let mock_provider = MockProvider::new(
        latest_blocks.clone().into_iter(),
        std::iter::empty(),
        logs.into_iter(),
        finalized_blocks,
        latest_blocks,
    );

    // spawn the watcher and verify received notifications are consistent.
    let mut l1_watcher = L1Watcher::spawn(mock_provider, None, Arc::new(config)).await;
    let mut prev_block_number = 0;
    let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(2));
    let _ = ticker.tick().await;

    loop {
        select! {
            notification = l1_watcher.recv() => {
                let notification = notification.map(|notif| (*notif).clone());
                if let Some(L1Notification::L1Message { block_number, .. }) = notification {
                    assert_ne!(prev_block_number, block_number, "indexed same block twice {block_number}");
                    prev_block_number = block_number
                }
            }
            _ = ticker.tick() => break
        }
    }

    Ok(())
}
