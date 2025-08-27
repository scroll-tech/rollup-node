//! Contains tests related to RN and EN sync.

use alloy_primitives::{b256, Address, U256};
use alloy_provider::{Provider, ProviderBuilder};
use futures::StreamExt;
use reqwest::Url;
use reth_network::{NetworkEvent, NetworkEventListenerProvider};
use reth_network_api::{events::PeerEvent, test_utils::PeersHandleProvider};
use reth_provider::{BlockIdReader, BlockReader};
use reth_scroll_chainspec::{SCROLL_DEV, SCROLL_SEPOLIA};
use reth_tokio_util::EventStream;
use rollup_node::{
    test_utils::{
        default_sequencer_test_scroll_rollup_node_config, default_test_scroll_rollup_node_config,
        setup_engine,
    },
    BeaconProviderArgs, ChainOrchestratorArgs, ConsensusArgs, DatabaseArgs, EngineDriverArgs,
    GasPriceOracleArgs, L1ProviderArgs, NetworkArgs, ScrollRollupNodeConfig, SequencerArgs,
};
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use rollup_node_manager::RollupManagerEvent;
use rollup_node_primitives::BlockInfo;
use rollup_node_providers::BlobSource;
use rollup_node_sequencer::L1MessageInclusionMode;
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use std::{path::PathBuf, sync::Arc};

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
        chain_orchestrator_args: ChainOrchestratorArgs {
            optimistic_sync_trigger: 100,
            ..Default::default()
        },
        l1_provider_args: L1ProviderArgs {
            url: Some(Url::parse(&format!("https://eth-sepolia.g.alchemy.com/v2/{alchemy_key}"))?),
            compute_units_per_second: 500,
            max_retries: 10,
            initial_backoff: 100,
        },
        engine_driver_args: EngineDriverArgs { sync_at_startup: false },
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
async fn test_should_trigger_pipeline_sync_for_execution_node() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = default_sequencer_test_scroll_rollup_node_config();

    // Create the chain spec for scroll mainnet with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_node_config.clone(), 1, chain_spec.clone(), false, false)
            .await
            .unwrap();
    let mut synced = nodes.pop().unwrap();
    let mut synced_events = synced.inner.rollup_manager_handle.get_event_listener().await?;

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec, false, false).await.unwrap();
    let mut unsynced = nodes.pop().unwrap();
    let mut unsynced_events = unsynced.inner.rollup_manager_handle.get_event_listener().await?;

    // Wait for the chain to be advanced by the sequencer.
    let optimistic_sync_trigger = node_config.chain_orchestrator_args.optimistic_sync_trigger + 1;
    wait_n_events(
        &mut synced_events,
        |e| matches!(e, RollupManagerEvent::BlockSequenced(_)),
        optimistic_sync_trigger,
    )
    .await;

    // Connect the nodes together.
    synced.network.add_peer(unsynced.network.record()).await;
    unsynced.network.next_session_established().await;
    synced.network.next_session_established().await;

    // Assert that the unsynced node triggers optimistic sync.
    wait_n_events(
        &mut unsynced_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(ChainOrchestratorEvent::OptimisticSync(
                    _
                ))
            )
        },
        1,
    )
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
    wait_n_events(
        &mut unsynced_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(ChainOrchestratorEvent::ChainExtended(
                    _
                ))
            )
        },
        1,
    )
    .await;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_should_consolidate_after_optimistic_sync() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = ScrollRollupNodeConfig {
        test: true,
        network_args: NetworkArgs {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
            sequencer_url: None,
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
        gas_price_oracle_args: GasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
    };

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Create a sequencer node and an unsynced node.
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_node_config, 1, chain_spec.clone(), false, false).await.unwrap();
    let mut sequencer = nodes.pop().unwrap();
    let sequencer_l1_watcher_tx = sequencer.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();
    let sequencer_handle = sequencer.inner.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_handle.get_event_listener().await?;

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec, false, false).await.unwrap();
    let mut follower = nodes.pop().unwrap();
    let follower_l1_watcher_tx = follower.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();
    let mut follower_events =
        follower.inner.add_ons_handle.rollup_manager_handle.get_event_listener().await?;

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
        wait_n_events(
            &mut sequencer_events,
            |e| {
                matches!(
                    e,
                    RollupManagerEvent::ChainOrchestratorEvent(
                        rollup_node_chain_orchestrator::ChainOrchestratorEvent::L1MessageCommitted(
                            _
                        )
                    )
                )
            },
            1,
        )
        .await;
        sequencer_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(i as u64))).await.unwrap();
        wait_n_events(
            &mut sequencer_events,
            |e| matches!(e, RollupManagerEvent::L1NotificationEvent(L1Notification::NewBlock(_))),
            1,
        )
        .await;
        sequencer_handle.build_block().await;
        wait_n_events(
            &mut sequencer_events,
            |e: RollupManagerEvent| matches!(e, RollupManagerEvent::BlockSequenced(_)),
            1,
        )
        .await;
    }

    // Connect the nodes together.
    sequencer.network.add_peer(follower.network.record()).await;
    follower.network.next_session_established().await;
    sequencer.network.next_session_established().await;

    // trigger a new block on the sequencer node.
    sequencer_handle.build_block().await;

    // Assert that the unsynced node triggers optimistic sync.
    wait_n_events(
        &mut follower_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(ChainOrchestratorEvent::OptimisticSync(
                    _
                ))
            )
        },
        1,
    )
    .await;

    // Let the unsynced node process the optimistic sync.
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Send all L1 messages to the unsynced node.
    for (i, l1_message) in l1_messages.iter().enumerate() {
        follower_l1_watcher_tx
            .send(Arc::new(L1Notification::L1Message {
                message: l1_message.clone(),
                block_number: i as u64,
                block_timestamp: i as u64 * 10,
            }))
            .await
            .unwrap();
        wait_n_events(
            &mut follower_events,
            |e: RollupManagerEvent| {
                matches!(
                    e,
                    RollupManagerEvent::ChainOrchestratorEvent(
                        rollup_node_chain_orchestrator::ChainOrchestratorEvent::L1MessageCommitted(
                            _
                        )
                    )
                )
            },
            1,
        )
        .await;
    }

    // Send a notification to the unsynced node that the L1 watcher is synced.
    follower_l1_watcher_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();

    // Wait for the unsynced node to sync to the L1 watcher.
    wait_n_events(
        &mut follower_events,
        |e| matches!(e, RollupManagerEvent::L1NotificationEvent(L1Notification::Synced)),
        1,
    )
    .await;

    // Let the unsynced node process the L1 messages.
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // build a new block on the sequencer node to trigger consolidation on the unsynced node.
    sequencer_handle.build_block().await;

    // Assert that the unsynced node consolidates the chain.
    wait_n_events(
        &mut follower_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(
                    ChainOrchestratorEvent::L2ChainCommitted(_, _, true)
                )
            )
        },
        1,
    )
    .await;

    // Now push a L1 message to the sequencer node and build a new block.
    sequencer_l1_watcher_tx
        .send(Arc::new(L1Notification::L1Message {
            message: TxL1Message {
                queue_index: 200,
                gas_limit: 21000,
                sender: Address::random(),
                to: Address::random(),
                value: U256::from(1),
                input: Default::default(),
            },
            block_number: 200,
            block_timestamp: 2010,
        }))
        .await
        .unwrap();
    wait_n_events(
        &mut sequencer_events,
        |e: RollupManagerEvent| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(
                    rollup_node_chain_orchestrator::ChainOrchestratorEvent::L1MessageCommitted(_)
                )
            )
        },
        1,
    )
    .await;
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(201))).await.unwrap();
    wait_n_events(
        &mut sequencer_events,
        |e| matches!(e, RollupManagerEvent::L1NotificationEvent(L1Notification::NewBlock(_))),
        1,
    )
    .await;
    sequencer_handle.build_block().await;

    wait_n_events(
        &mut follower_events,
        |e| matches!(e, RollupManagerEvent::NewBlockReceived(_)),
        1,
    )
    .await;

    // Assert that the follower node does not accept the new block as it does not have the L1
    // message.
    wait_n_events(
        &mut follower_events,
        |e| matches!(e, RollupManagerEvent::L1MessageMissingInDatabase { start: _ }),
        1,
    )
    .await;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_consolidation() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = ScrollRollupNodeConfig {
        test: true,
        network_args: NetworkArgs {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
            sequencer_url: None,
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
        gas_price_oracle_args: GasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
    };

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Create a sequencer node and an unsynced node.
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_node_config, 1, chain_spec.clone(), false, false).await.unwrap();
    let mut sequencer = nodes.pop().unwrap();
    let sequencer_l1_watcher_tx = sequencer.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();
    let sequencer_handle = sequencer.inner.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_handle.get_event_listener().await?;

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec, false, false).await.unwrap();
    let mut follower = nodes.pop().unwrap();
    let follower_l1_watcher_tx = follower.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();
    let mut follower_events = follower.inner.rollup_manager_handle.get_event_listener().await?;

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
    wait_n_events(
        &mut sequencer_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(
                    rollup_node_chain_orchestrator::ChainOrchestratorEvent::L1MessageCommitted(_)
                )
            )
        },
        1,
    )
    .await;
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(2))).await.unwrap();

    follower_l1_watcher_tx
        .send(Arc::new(L1Notification::L1Message {
            message: l1_message,
            block_number: 0,
            block_timestamp: 0,
        }))
        .await
        .unwrap();
    wait_n_events(
        &mut follower_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(
                    rollup_node_chain_orchestrator::ChainOrchestratorEvent::L1MessageCommitted(_)
                )
            )
        },
        1,
    )
    .await;

    // Send a notification to both nodes that the L1 watcher is synced.
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();
    follower_l1_watcher_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();

    // Build a new block on the sequencer node.
    sequencer_handle.build_block().await;

    // Assert that the unsynced node consolidates the chain.
    wait_n_events(
        &mut follower_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(
                    ChainOrchestratorEvent::L2ChainCommitted(_, _, true)
                )
            )
        },
        1,
    )
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
    wait_n_events(
        &mut sequencer_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(
                    rollup_node_chain_orchestrator::ChainOrchestratorEvent::L1MessageCommitted(_)
                )
            )
        },
        1,
    )
    .await;
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(5))).await.unwrap();
    sequencer_handle.build_block().await;

    // Assert that the follower node rejects the new block as it hasn't received the L1 message.
    wait_n_events(
        &mut follower_events,
        |e| matches!(e, RollupManagerEvent::L1MessageMissingInDatabase { start: _ }),
        1,
    )
    .await;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_chain_orchestrator_shallow_reorg_with_gap() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = ScrollRollupNodeConfig {
        test: true,
        network_args: NetworkArgs {
            enable_eth_scroll_wire_bridge: false,
            enable_scroll_wire: true,
            ..Default::default()
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
        gas_price_oracle_args: GasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
    };

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Create a sequencer node and an unsynced node.
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_node_config.clone(), 1, chain_spec.clone(), false, false)
            .await
            .unwrap();
    let mut sequencer = nodes.pop().unwrap();
    let sequencer_handle = sequencer.inner.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_handle.get_event_listener().await?;

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec.clone(), false, false).await.unwrap();
    let mut follower = nodes.pop().unwrap();
    let mut follower_events = follower.inner.rollup_manager_handle.get_event_listener().await?;

    // Connect the nodes together.
    sequencer.connect(&mut follower).await;

    // initially the sequencer should build 100 empty blocks and the follower should follow them
    let mut reorg_block_info = BlockInfo::default();
    for i in 0..100 {
        sequencer_handle.build_block().await;
        wait_n_events(
            &mut sequencer_events,
            |e| {
                if let RollupManagerEvent::BlockSequenced(block) = e {
                    if i == 95 {
                        reorg_block_info = (&block).into();
                    }
                    true
                } else {
                    false
                }
            },
            1,
        )
        .await;
        wait_n_events(
            &mut follower_events,
            |e| {
                matches!(
                    e,
                    RollupManagerEvent::ChainOrchestratorEvent(
                        ChainOrchestratorEvent::L2ChainCommitted(_, _, _)
                    )
                )
            },
            1,
        )
        .await;
    }

    println!("produced 100 blocks");

    // disconnect the two nodes
    let mut sequencer_network_events = sequencer.inner.network.event_listener();
    let mut follower_network_events = follower.inner.network.event_listener();
    sequencer.inner.network.peers_handle().remove_peer(follower.network.record().id);
    while let Some(ev) = sequencer_network_events.next().await {
        if let NetworkEvent::Peer(PeerEvent::SessionClosed { peer_id: _, reason: _ }) = ev {
            break
        }
    }
    while let Some(ev) = sequencer_network_events.next().await {
        if let NetworkEvent::Peer(PeerEvent::PeerRemoved(_)) = ev {
            break
        }
    }
    while let Some(ev) = follower_network_events.next().await {
        if let NetworkEvent::Peer(PeerEvent::SessionClosed { peer_id: _, reason: _ }) = ev {
            break
        }
    }

    sequencer_handle.update_fcs_head(reorg_block_info).await;

    // Have the sequencer build 2 new blocks, one containing the L1 message.
    sequencer_handle.build_block().await;
    wait_n_events(&mut sequencer_events, |e| matches!(e, RollupManagerEvent::BlockSequenced(_)), 1)
        .await;
    sequencer_handle.build_block().await;
    wait_n_events(&mut sequencer_events, |e| matches!(e, RollupManagerEvent::BlockSequenced(_)), 1)
        .await;

    // connect the two nodes again
    follower.connect(&mut sequencer).await;

    // now build a final block
    sequencer_handle.build_block().await;

    // Wait for the follower node to reorg to the new chain.
    wait_n_events(
        &mut follower_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(ChainOrchestratorEvent::ChainReorged(_))
            )
        },
        1,
    )
    .await;

    Ok(())
}

/// Waits for n events to be emitted.
async fn wait_n_events(
    events: &mut EventStream<RollupManagerEvent>,
    mut matches: impl FnMut(RollupManagerEvent) -> bool,
    mut n: u64,
) {
    while let Some(event) = events.next().await {
        if matches(event) {
            n -= 1;
        }
        if n == 0 {
            break
        }
    }
}
