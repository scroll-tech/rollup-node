//! Contains tests related to RN and EN sync.

use alloy_primitives::{Address, U256};
use alloy_provider::{Provider, ProviderBuilder};
use futures::StreamExt;
use reth_e2e_test_utils::NodeHelperType;
use reth_scroll_chainspec::SCROLL_DEV;
use rollup_node::{
    test_utils::{
        default_sequencer_test_scroll_rollup_node_config, default_test_scroll_rollup_node_config,
        setup_engine,
    },
    BeaconProviderArgs, ChainOrchestratorArgs, DatabaseArgs, EngineDriverArgs, L1ProviderArgs,
    NetworkArgs, ScrollRollupNode, ScrollRollupNodeConfig, SequencerArgs,
};
use rollup_node_manager::RollupManagerEvent;
use rollup_node_sequencer::L1MessageInclusionMode;
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use std::{path::PathBuf, sync::Arc};

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
    let optimistic_sync_trigger = node_config.chain_orchestrator_args.optimistic_sync_trigger + 1;
    wait_n_events(
        &synced,
        |e| matches!(e, RollupManagerEvent::BlockSequenced(_)),
        optimistic_sync_trigger,
    )
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
        if retries > 10 || num > optimistic_sync_trigger {
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

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_should_consolidate_after_optimistic_sync() {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = ScrollRollupNodeConfig {
        test: true,
        network_args: NetworkArgs { enable_eth_scroll_wire_bridge: true, enable_scroll_wire: true },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs::default(),
        signer_args: Default::default(),
    };

    // Create the chain spec for scroll dev with Euclid v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Create a sequencer node and an unsynced node.
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_node_config, 1, chain_spec.clone(), false).await.unwrap();
    let mut sequencer = nodes.pop().unwrap();
    let sequencer_l1_watcher_tx = sequencer.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();
    let sequencer_handle = sequencer.inner.rollup_manager_handle.clone();

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec, false).await.unwrap();
    let mut unsynced = nodes.pop().unwrap();
    let unsynced_l1_watcher_tx = unsynced.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();

    // Create a sequence of L1 messages to be added to the sequencer node.
    const L1_MESSAGES_COUNT: usize = 200;
    let mut l1_messages = Vec::with_capacity(L1_MESSAGES_COUNT);
    for i in 0..L1_MESSAGES_COUNT as u64 {
        let l1_message = TxL1Message {
            queue_index: i,
            gas_limit: 21000,
            sender: Address::random(),
            to: Address::random(),
            value: U256::from(1),
            input: Default::default(),
        };
        l1_messages.push(l1_message);
    }

    // Add the L1 messages to the sequencer node.
    for (i, l1_message) in l1_messages.iter().enumerate() {
        sequencer_l1_watcher_tx
            .send(Arc::new(L1Notification::L1Message {
                message: l1_message.clone(),
                block_number: i as u64,
                block_timestamp: i as u64 * 10,
            }))
            .await
            .unwrap();
        wait_n_events(&sequencer, |e| matches!(e, RollupManagerEvent::L1MessageIndexed(_)), 1)
            .await;
        sequencer_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(i as u64))).await.unwrap();
        sequencer_handle.build_block().await;
        wait_n_events(
            &sequencer,
            |e: RollupManagerEvent| matches!(e, RollupManagerEvent::BlockSequenced(_)),
            1,
        )
        .await;
    }

    // Connect the nodes together.
    sequencer.network.add_peer(unsynced.network.record()).await;
    unsynced.network.next_session_established().await;
    sequencer.network.next_session_established().await;

    // trigger a new block on the sequencer node.
    sequencer_handle.build_block().await;

    // Assert that the unsynced node triggers optimistic sync.
    wait_n_events(&unsynced, |e| matches!(e, RollupManagerEvent::OptimisticSyncTriggered(_)), 1)
        .await;

    // Let the unsynced node process the optimistic sync.
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Send all L1 messages to the unsynced node.
    for (i, l1_message) in l1_messages.iter().enumerate() {
        unsynced_l1_watcher_tx
            .send(Arc::new(L1Notification::L1Message {
                message: l1_message.clone(),
                block_number: i as u64,
                block_timestamp: i as u64 * 10,
            }))
            .await
            .unwrap();
        wait_n_events(
            &unsynced,
            |e: RollupManagerEvent| matches!(e, RollupManagerEvent::L1MessageIndexed(_)),
            1,
        )
        .await;
    }

    // Send a notification to the unsynced node that the L1 watcher is synced.
    unsynced_l1_watcher_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();

    // Wait for the unsynced node to sync to the L1 watcher.
    wait_n_events(&unsynced, |e| matches!(e, RollupManagerEvent::L1Synced), 1).await;

    // Let the unsynced node process the L1 messages.
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // build a new block on the sequencer node to trigger consolidation on the unsynced node.
    sequencer_handle.build_block().await;

    // Assert that the unsynced node consolidates the chain.
    wait_n_events(&unsynced, |e| matches!(e, RollupManagerEvent::L2ChainCommitted(_, _, true)), 1)
        .await;

    // Now push a L1 message to the sequencer node and build a new block.
    unsynced_l1_watcher_tx
        .send(Arc::new(L1Notification::L1Message {
            message: TxL1Message {
                queue_index: 200,
                gas_limit: 21000,
                sender: Address::random(),
                to: Address::random(),
                value: U256::from(1),
                input: Default::default(),
            },
            block_number: 201,
            block_timestamp: 2010,
        }))
        .await
        .unwrap();
    sequencer_handle.build_block().await;

    wait_n_events(&unsynced, |e| matches!(e, RollupManagerEvent::NewBlockReceived(_)), 1).await;

    // Assert that the follower node does not accept the new block as it does not have the L1
    // message.
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_consolidation() {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = ScrollRollupNodeConfig {
        test: true,
        network_args: NetworkArgs { enable_eth_scroll_wire_bridge: true, enable_scroll_wire: true },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs::default(),
        signer_args: Default::default(),
    };

    // Create the chain spec for scroll dev with Euclid v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Create a sequencer node and an unsynced node.
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_node_config, 1, chain_spec.clone(), false).await.unwrap();
    let mut sequencer = nodes.pop().unwrap();
    let sequencer_l1_watcher_tx = sequencer.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();
    let sequencer_handle = sequencer.inner.rollup_manager_handle.clone();

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec, false).await.unwrap();
    let mut follower = nodes.pop().unwrap();
    let follower_l1_watcher_tx = follower.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();

    // Connect the nodes together.
    sequencer.network.add_peer(follower.network.record()).await;
    follower.network.next_session_established().await;
    sequencer.network.next_session_established().await;

    // Create a L1 message and send it to both nodes.
    let l1_message = TxL1Message {
        queue_index: 0,
        gas_limit: 21000,
        sender: Address::random(),
        to: Address::random(),
        value: U256::from(1),
        input: Default::default(),
    };
    sequencer_l1_watcher_tx
        .send(Arc::new(L1Notification::L1Message {
            message: l1_message.clone(),
            block_number: 0,
            block_timestamp: 0,
        }))
        .await
        .unwrap();
    wait_n_events(&sequencer, |e| matches!(e, RollupManagerEvent::L1MessageIndexed(_)), 1).await;
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(2))).await.unwrap();

    follower_l1_watcher_tx
        .send(Arc::new(L1Notification::L1Message {
            message: l1_message,
            block_number: 0,
            block_timestamp: 0,
        }))
        .await
        .unwrap();
    wait_n_events(&follower, |e| matches!(e, RollupManagerEvent::L1MessageIndexed(_)), 1).await;

    // Send a notification to both nodes that the L1 watcher is synced.
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();
    follower_l1_watcher_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();

    // Build a new block on the sequencer node.
    sequencer_handle.build_block().await;

    // Assert that the unsynced node consolidates the chain.
    wait_n_events(&follower, |e| matches!(e, RollupManagerEvent::L2ChainCommitted(_, _, true)), 1)
        .await;

    // Now push a L1 message to the sequencer node and build a new block.
    sequencer_l1_watcher_tx
        .send(Arc::new(L1Notification::L1Message {
            message: TxL1Message {
                queue_index: 1,
                gas_limit: 21000,
                sender: Address::random(),
                to: Address::random(),
                value: U256::from(1),
                input: Default::default(),
            },
            block_number: 1,
            block_timestamp: 10,
        }))
        .await
        .unwrap();
    wait_n_events(&sequencer, |e| matches!(e, RollupManagerEvent::L1MessageIndexed(_)), 1).await;
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(5))).await.unwrap();
    sequencer_handle.build_block().await;

    // Assert that the follower node rejects the new block as it hasn't received the L1 message.
    wait_n_events(
        &follower,
        |e| matches!(e, RollupManagerEvent::L1MessageMissingInDatabase { start: _ }),
        1,
    )
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
