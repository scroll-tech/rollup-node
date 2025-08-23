//! Contains tests related to RN and EN sync.

use alloy_primitives::b256;
use alloy_provider::{Provider, ProviderBuilder};
use futures::StreamExt;
use reqwest::Url;
use reth_e2e_test_utils::NodeHelperType;
use reth_provider::{BlockIdReader, BlockReader};
use reth_scroll_chainspec::{SCROLL_DEV, SCROLL_SEPOLIA};
use rollup_node::{
    test_utils::{
        default_sequencer_test_scroll_rollup_node_config, default_test_scroll_rollup_node_config,
        setup_engine,
    },
    BeaconProviderArgs, ConsensusArgs, DatabaseArgs, EngineDriverArgs, GasPriceOracleArgs,
    L1ProviderArgs, NetworkArgs, ScrollRollupNode, ScrollRollupNodeConfig, SequencerArgs,
};
use rollup_node_manager::{RollupManagerCommand, RollupManagerEvent};
use rollup_node_providers::BlobSource;
use tokio::sync::oneshot;

#[tokio::test]
async fn test_should_consolidate_to_block_15k() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Prepare the config for a L1 consolidation.
    let alchemy_key = if let Ok(key) = std::env::var("ALCHEMY_KEY") {
        key
    } else {
        eprintln!("ALCHEMY_KEY environment variable is not set. Skipping test.");
        return Ok(());
    };

    let node_config = ScrollRollupNodeConfig {
        test: false,
        network_args: NetworkArgs {
            enable_eth_scroll_wire_bridge: false,
            enable_scroll_wire: false,
            sequencer_url: None,
        },
        database_args: DatabaseArgs::default(),
        l1_provider_args: L1ProviderArgs {
            url: Some(Url::parse(&format!("https://eth-sepolia.g.alchemy.com/v2/{alchemy_key}"))?),
            compute_units_per_second: 500,
            max_retries: 10,
            initial_backoff: 100,
        },
        engine_driver_args: EngineDriverArgs {
            en_sync_trigger: 10000000000,
            sync_at_startup: false,
        },
        sequencer_args: SequencerArgs { sequencer_enabled: false, ..Default::default() },
        beacon_provider_args: BeaconProviderArgs {
            url: Some(Url::parse("https://eth-beacon-chain.drpc.org/rest/")?),
            compute_units_per_second: 100,
            max_retries: 10,
            initial_backoff: 100,
            blob_source: BlobSource::Beacon,
        },
        signer_args: Default::default(),
        gas_price_oracle_args: GasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
    };

    let chain_spec = (*SCROLL_SEPOLIA).clone();
    let (mut nodes, _tasks, _) =
        setup_engine(node_config, 1, chain_spec.clone(), false, false).await?;
    let node = nodes.pop().unwrap();

    // We perform consolidation up to block 15k. This allows us to capture a batch revert event at
    // block 11419 (batch 1653).
    while node.inner.provider.safe_block_num_hash()?.map(|x| x.number).unwrap_or_default() < 15000 {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await
    }

    let block_hash_15k = node.inner.provider.block(15000.into())?.unwrap();

    assert_eq!(
        block_hash_15k.hash_slow(),
        b256!("86901ebce1840ee45c1d5c70bf85ce6924f7a066ef11575d0f381858c83845d4")
    );

    Ok(())
}

/// We test if the syncing of the RN is correctly triggered and released when the EN reaches sync.
#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_should_trigger_pipeline_sync_for_execution_node() {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let mut sequencer_node_config = default_sequencer_test_scroll_rollup_node_config();
    sequencer_node_config.sequencer_args.block_time = 40;

    // Create the chain spec for scroll mainnet with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_node_config.clone(), 1, chain_spec.clone(), false, false)
            .await
            .unwrap();
    let mut synced = nodes.pop().unwrap();

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec, false, false).await.unwrap();
    let mut unsynced = nodes.pop().unwrap();

    // Wait for the chain to be advanced by the sequencer.
    let en_sync_trigger = node_config.engine_driver_args.en_sync_trigger + 1;
    wait_n_events(&synced, |e| matches!(e, RollupManagerEvent::BlockSequenced(_)), en_sync_trigger)
        .await;

    // Connect the nodes together.
    synced.network.add_peer(unsynced.network.record()).await;
    unsynced.network.next_session_established().await;
    synced.network.next_session_established().await;

    // Wait for the unsynced node to receive a block.
    wait_n_events(&unsynced, |e| matches!(e, RollupManagerEvent::NewBlockReceived(_)), 1).await;

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
    wait_n_events(&unsynced, |e| matches!(e, RollupManagerEvent::BlockImported(_)), 1).await;

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
