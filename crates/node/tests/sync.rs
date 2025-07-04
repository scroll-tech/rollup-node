//! Contains tests related to RN and EN sync.

use alloy_provider::{Provider, ProviderBuilder};
use futures::StreamExt;
use reth_e2e_test_utils::NodeHelperType;
use reth_scroll_chainspec::SCROLL_DEV;
use rollup_node::{
    test_utils::{
        default_sequencer_test_scroll_rollup_node_config, default_test_scroll_rollup_node_config,
        setup_engine,
    },
    ScrollRollupNode,
};
use rollup_node_manager::RollupManagerEvent;

/// We test if the syncing of the RN is correctly triggered and released when the EN reaches sync.
#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_should_trigger_pipeline_sync_for_execution_node() {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = default_sequencer_test_scroll_rollup_node_config();

    // Create the chain spec for scroll mainnet with Euclid v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_node_config.clone(), 1, chain_spec.clone(), false).await.unwrap();
    let mut synced = nodes.pop().unwrap();

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec, false).await.unwrap();
    let mut unsynced = nodes.pop().unwrap();

    // Wait for the chain to be advanced by the sequencer.
    let en_sync_trigger = node_config.engine_driver_args.en_sync_trigger + 1;
    wait_n_events(&synced, |e| matches!(e, RollupManagerEvent::BlockSequenced(_)), en_sync_trigger)
        .await;

    // Connect the nodes together.
    synced.network.add_peer(unsynced.network.record()).await;
    unsynced.network.next_session_established().await;
    synced.network.next_session_established().await;

    // Assert that the unsynced node triggers optimistic sync.
    wait_n_events(&unsynced, |e| matches!(e, RollupManagerEvent::OptimisticSyncTriggered(_)), 1)
        .await;

    // Verify the unsynced node syncs.
    let provider = ProviderBuilder::new().connect_http(unsynced.rpc_url());
    let mut retries = 0;
    let mut num = provider.get_block_number().await.unwrap();

    loop {
        if retries > 10 || num > en_sync_trigger {
            break
        }
        num = provider.get_block_number().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        retries += 1;
    }

    // Assert that the unsynced node triggers a chain extension on the optimistic chain.
    wait_n_events(&unsynced, |e| matches!(e, RollupManagerEvent::ChainExtensionTriggered(_)), 1)
        .await;
}

/// Waits for n events to be emitted.
async fn wait_n_events(
    node: &NodeHelperType<ScrollRollupNode>,
    matches: impl Fn(RollupManagerEvent) -> bool,
    mut n: u64,
) {
    let mut events = node.inner.rollup_manager_handle.get_event_listener().await.unwrap();
    while let Some(event) = events.next().await {
        if matches(event) {
            n -= 1;
        }
        if n == 0 {
            break
        }
    }
}
