//! Contains tests related to RN and EN sync.

use alloy_provider::{Provider, ProviderBuilder};
use reth_scroll_chainspec::SCROLL_DEV;
use rollup_node::test_utils::{
    default_sequencer_test_scroll_rollup_node_config, default_test_scroll_rollup_node_config,
    setup_engine,
};
use rollup_node_manager::RollupManagerCommand;
use tokio::sync::oneshot;

/// We test if the syncing of the RN is correctly triggered and released when the EN reaches sync.
#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_should_trigger_pipeline_sync_for_execution_node() {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = default_sequencer_test_scroll_rollup_node_config();
    let block_time = sequencer_node_config.sequencer_args.block_time;

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
    tokio::time::sleep(tokio::time::Duration::from_millis(en_sync_trigger * block_time)).await;

    // Connect the nodes together.
    synced.network.add_peer(unsynced.network.record()).await;
    unsynced.network.next_session_established().await;
    synced.network.next_session_established().await;

    // Wait for the unsynced node to receive a block.
    tokio::time::sleep(tokio::time::Duration::from_millis(block_time)).await;

    // Check the unsynced node enters sync mode.
    let (tx, rx) = oneshot::channel();
    unsynced
        .inner
        .add_ons_handle
        .rollup_manager_handle
        .send_command(RollupManagerCommand::Status(tx))
        .await;
    let status = rx.await.unwrap();
    assert!(status.syncing);

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

    // Wait at least a single block for the driver to exit sync mode.
    tokio::time::sleep(tokio::time::Duration::from_millis(block_time)).await;

    // Check the unsynced node exits sync mode.
    let (tx, rx) = oneshot::channel();
    unsynced
        .inner
        .add_ons_handle
        .rollup_manager_handle
        .send_command(RollupManagerCommand::Status(tx))
        .await;
    let status = rx.await.unwrap();
    assert!(!status.syncing);
}
