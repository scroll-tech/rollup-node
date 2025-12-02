//! Contains tests related to RN and EN sync.

use alloy_primitives::{b256, Address, B256, U256};
use futures::StreamExt;
use reqwest::Url;
use reth_provider::{BlockIdReader, BlockReader};
use reth_rpc_eth_api::helpers::EthTransactions;
use reth_scroll_chainspec::{SCROLL_DEV, SCROLL_SEPOLIA};
use reth_tokio_util::EventStream;
use rollup_node::{
    test_utils::{
        default_test_scroll_rollup_node_config, generate_tx, setup_engine, EventAssertions,
        TestFixture,
    },
    BlobProviderArgs, ChainOrchestratorArgs, ConsensusArgs, EngineDriverArgs, L1ProviderArgs,
    RollupNodeDatabaseArgs, RollupNodeGasPriceOracleArgs, RollupNodeNetworkArgs, RpcArgs,
    ScrollRollupNodeConfig, SequencerArgs, TestArgs,
};
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use rollup_node_primitives::BlockInfo;
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
        test_args: TestArgs { test: false, skip_l1_synced: false },
        network_args: RollupNodeNetworkArgs {
            enable_eth_scroll_wire_bridge: false,
            enable_scroll_wire: false,
            sequencer_url: None,
            signer_address: None,
        },
        database_args: RollupNodeDatabaseArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs {
            optimistic_sync_trigger: 100,
            ..Default::default()
        },
        l1_provider_args: L1ProviderArgs {
            url: Some(Url::parse(&format!("https://eth-sepolia.g.alchemy.com/v2/{alchemy_key}"))?),
            compute_units_per_second: 500,
            max_retries: 10,
            initial_backoff: 100,
            logs_query_block_range: 500,
            cache_max_items: 100,
        },
        engine_driver_args: EngineDriverArgs { sync_at_startup: false },
        sequencer_args: SequencerArgs {
            sequencer_enabled: false,
            allow_empty_blocks: true,
            ..Default::default()
        },
        blob_provider_args: BlobProviderArgs {
            s3_url: Some(Url::parse(
                "https://scroll-sepolia-blob-data.s3.us-west-2.amazonaws.com/",
            )?),
            compute_units_per_second: 100,
            max_retries: 10,
            initial_backoff: 100,
            ..Default::default()
        },
        signer_args: Default::default(),
        gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
        rpc_args: RpcArgs::default(),
    };

    let chain_spec = (*SCROLL_SEPOLIA).clone();
    let (mut nodes, _databases, _tasks, _) =
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

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_node_produces_block_on_startup() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Start a sequencer and follower node.
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .auto_start(true)
        .allow_empty_blocks(false)
        .build()
        .await?;

    fixture.l1().sync().await?;

    // wait for both nodes to be synced.
    fixture.expect_event_on_all_nodes().chain_consolidated().await?;

    // construct a transaction and send it to the follower node.
    let wallet = fixture.wallet();
    let follower_rpc = fixture.follower(0).node.rpc.inner.clone();
    let handle = tokio::spawn(async move {
        loop {
            let tx = generate_tx(wallet.clone()).await;
            let _ = follower_rpc.eth_api().send_raw_transaction(tx).await;
        }
    });

    fixture.expect_event_on_followers().chain_extended(1).await?;
    drop(handle);

    Ok(())
}

/// We test if the syncing of the RN is correctly triggered and released when the EN reaches sync.
#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_should_trigger_pipeline_sync_for_execution_node() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    const OPTIMISTIC_SYNC_TRIGGER: u64 = 100;
    let mut sequencer = TestFixture::builder()
        .sequencer()
        .block_time(40)
        .auto_start(true)
        .optimistic_sync_trigger(OPTIMISTIC_SYNC_TRIGGER)
        .build()
        .await?;

    let mut follower = TestFixture::builder()
        .followers(1)
        .optimistic_sync_trigger(OPTIMISTIC_SYNC_TRIGGER)
        .build()
        .await?;

    // Set the L1 to synced on the synced node to start block production.
    sequencer.l1().sync().await?;

    // Wait for the chain to be advanced by the sequencer.
    sequencer.expect_event().block_sequenced(OPTIMISTIC_SYNC_TRIGGER + 1).await?;

    // Connect the nodes together.
    sequencer.sequencer().node.connect(&mut follower.follower(0).node).await;

    // Assert that the unsynced node triggers optimistic sync.
    follower.expect_event().optimistic_sync().await?;

    // Verify the unsynced node syncs.
    let mut num = follower.get_block(0).await?.header.number;
    let mut retries = 0;

    loop {
        if retries > 10 || num > OPTIMISTIC_SYNC_TRIGGER {
            break
        }
        num = follower.get_block(0).await?.header.number;
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        retries += 1;
    }

    // Assert that the unsynced node triggers a chain extension on the optimistic chain.
    follower.expect_event().chain_extended(num).await?;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_should_consolidate_after_optimistic_sync() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut sequencer = TestFixture::builder()
        .sequencer()
        .with_eth_scroll_bridge(true)
        .with_scroll_wire(true)
        .auto_start(true)
        .block_time(20)
        .with_l1_message_delay(0)
        .allow_empty_blocks(true)
        .build()
        .await?;

    let mut follower = TestFixture::builder().followers(1).build().await?;

    // Send a notification to the sequencer node that the L1 watcher is synced.
    sequencer.l1().sync().await?;

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
        sequencer
            .l1()
            .add_message()
            .to(l1_message.to)
            .queue_index(l1_message.queue_index)
            .gas_limit(l1_message.gas_limit)
            .sender(l1_message.sender)
            .value(l1_message.value)
            .input(l1_message.input.clone())
            .at_block(i as u64)
            .send()
            .await?;
        sequencer.expect_event().l1_message_committed().await?;

        sequencer.l1().new_block(i as u64).await?;
        sequencer.expect_event().new_l1_block().await?;

        sequencer.build_block().expect_block_number((i + 1) as u64).build_and_await_block().await?;
    }

    // Connect the nodes together.
    sequencer.sequencer().node.connect(&mut follower.follower(0).node).await;

    // trigger a new block on the sequencer node.
    sequencer.build_block().build_and_await_block().await?;

    // Assert that the unsynced node triggers optimistic sync.
    follower.expect_event().optimistic_sync().await?;

    // Let the unsynced node process the optimistic sync.
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Send all L1 messages to the unsynced node.
    for (i, l1_message) in l1_messages.iter().enumerate() {
        follower
            .l1()
            .add_message()
            .to(l1_message.to)
            .queue_index(l1_message.queue_index)
            .gas_limit(l1_message.gas_limit)
            .sender(l1_message.sender)
            .value(l1_message.value)
            .input(l1_message.input.clone())
            .at_block(i as u64)
            .send()
            .await?;
        follower.expect_event().l1_message_committed().await?;
    }

    // Send a notification to the unsynced node that the L1 watcher is synced.
    follower.l1().sync().await?;

    // Wait for the unsynced node to sync to the L1 watcher.
    follower.expect_event().l1_synced().await?;

    // Let the unsynced node process the L1 messages.
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // build a new block on the sequencer node to trigger consolidation on the unsynced node.
    sequencer.build_block().build_and_await_block().await?;

    // Assert that the unsynced node consolidates the chain.
    follower.expect_event().chain_extended((L1_MESSAGES_COUNT + 2) as u64).await?;

    // Now push a L1 message to the sequencer node and build a new block.
    sequencer
        .l1()
        .add_message()
        .queue_index(200)
        .sender(Address::random())
        .value(1)
        .at_block(200)
        .send()
        .await?;
    sequencer.expect_event().l1_message_committed().await?;

    sequencer.l1().new_block(201).await?;
    sequencer.expect_event().new_l1_block().await?;

    sequencer.build_block().build_and_await_block().await?;
    follower.expect_event().new_block_received().await?;

    // Assert that the follower node does not accept the new block as it does not have the L1
    // message.
    follower
        .expect_event()
        .where_event(|e| matches!(e, ChainOrchestratorEvent::L1MessageNotFoundInDatabase(_)))
        .await?;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_consolidation() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut sequencer = TestFixture::builder()
        .sequencer()
        .with_eth_scroll_bridge(true)
        .with_scroll_wire(true)
        .auto_start(false)
        .block_time(10)
        .with_l1_message_delay(0)
        .allow_empty_blocks(true)
        .build()
        .await?;

    let mut follower = TestFixture::builder().followers(1).build().await?;

    // Connect the nodes together.
    sequencer.sequencer().node.connect(&mut follower.follower(0).node).await;

    // Create a L1 message and send it to both nodes.
    let sender = Address::random();
    let to = Address::random();

    sequencer.l1().add_message().sender(sender).to(to).value(1).queue_index(0).send().await?;
    sequencer.expect_event().l1_message_committed().await?;

    follower.l1().add_message().sender(sender).to(to).value(1).send().await?;
    follower.expect_event().l1_message_committed().await?;

    // Send a notification to both nodes that the L1 watcher is synced.
    sequencer.l1().sync().await?;
    follower.l1().sync().await?;

    // Assert that the unsynced node consolidates the chain.
    follower.expect_event().chain_consolidated().await?;

    // Build a new block on the sequencer node.
    sequencer.build_block().build_and_await_block().await?;

    // Now push a L1 message to the sequencer node and build a new block.
    sequencer
        .l1()
        .add_message()
        .sender(Address::random())
        .to(Address::random())
        .value(1)
        .queue_index(1)
        .at_block(1)
        .send()
        .await?;
    sequencer.expect_event().l1_message_committed().await?;

    sequencer.l1().new_block(5).await?;
    sequencer.expect_event().new_l1_block().await?;
    sequencer.build_block().build_and_await_block().await?;

    // Assert that the follower node rejects the new block as it hasn't received the L1 message.
    follower
        .expect_event()
        .where_event(|e| matches!(e, ChainOrchestratorEvent::L1MessageNotFoundInDatabase(_)))
        .await?;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_chain_orchestrator_reorg_with_gap_above_head() -> eyre::Result<()> {
    test_chain_orchestrator_fork_choice(100, Some(95), 20, |e| {
        if let ChainOrchestratorEvent::ChainReorged(chain_import) = e {
            // Assert that the chain import is as expected.
            assert_eq!(chain_import.chain.len(), 21);
            true
        } else {
            false
        }
    })
    .await
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_chain_orchestrator_reorg_with_gap_below_head() -> eyre::Result<()> {
    test_chain_orchestrator_fork_choice(100, Some(50), 20, |e| {
        if let ChainOrchestratorEvent::ChainReorged(chain_import) = e {
            // Assert that the chain import is as expected.
            assert_eq!(chain_import.chain.len(), 21);
            true
        } else {
            false
        }
    })
    .await
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_chain_orchestrator_extension_with_gap() -> eyre::Result<()> {
    test_chain_orchestrator_fork_choice(100, None, 20, |e| {
        if let ChainOrchestratorEvent::ChainExtended(chain_import) = e {
            // Assert that the chain import is as expected.
            assert_eq!(chain_import.chain.len(), 21);
            true
        } else {
            false
        }
    })
    .await
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_chain_orchestrator_extension_no_gap() -> eyre::Result<()> {
    test_chain_orchestrator_fork_choice(100, None, 0, |e| {
        if let ChainOrchestratorEvent::ChainExtended(chain_import) = e {
            // Assert that the chain import is as expected.
            assert_eq!(chain_import.chain.len(), 1);
            true
        } else {
            false
        }
    })
    .await
}

#[allow(clippy::large_stack_frames)]
async fn test_chain_orchestrator_fork_choice(
    initial_blocks: usize,
    reorg_block_number: Option<usize>,
    additional_blocks: usize,
    expected_final_event_predicate: impl Fn(&ChainOrchestratorEvent) -> bool,
) -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut sequencer = TestFixture::builder()
        .sequencer()
        .with_scroll_wire(true)
        .with_eth_scroll_bridge(false)
        .auto_start(false)
        .block_time(10)
        .with_l1_message_delay(0)
        .allow_empty_blocks(true)
        .build()
        .await?;

    let mut follower = TestFixture::builder().followers(1).build().await?;

    // Connect the nodes together.
    sequencer.sequencer().node.connect(&mut follower.follower(0).node).await;

    // set both the sequencer and follower L1 watchers to synced
    sequencer.l1().sync().await?;
    follower.l1().sync().await?;

    // Initially the sequencer should build 100 empty blocks in each and the follower
    // should follow them
    let mut reorg_block_info: Option<BlockInfo> = None;
    for i in 0..initial_blocks {
        let num = (i + 1) as u64;
        let block = sequencer.build_block().build_and_await_block().await?;

        if Some(i) == reorg_block_number {
            reorg_block_info = Some((&block).into());
        }

        follower.expect_event().chain_extended(num).await?;
    }

    // Now reorg the sequencer and disable gossip so we can create fork
    let sequencer_handle = &sequencer.sequencer().rollup_manager_handle;
    sequencer_handle.set_gossip(false).await?;
    if let Some(block_info) = reorg_block_info {
        sequencer_handle.update_fcs_head(block_info).await?;
    }

    // wait two seconds to ensure the timestamp of the new blocks is greater than the old ones
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Have the sequencer build 20 new blocks, containing new L1 messages.
    for _ in 0..additional_blocks {
        sequencer.build_block().build_and_await_block().await?;
    }

    // now build a final block
    let sequencer_handle = &sequencer.sequencer().rollup_manager_handle;
    sequencer_handle.set_gossip(true).await?;
    sequencer.build_block().build_and_await_block().await?;

    // Wait for the follower node to accept the new chain
    follower.expect_event().where_event(expected_final_event_predicate).await?;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_chain_orchestrator_l1_reorg() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();
    let sequencer_node_config = ScrollRollupNodeConfig {
        test_args: TestArgs { test: true, skip_l1_synced: false },
        network_args: RollupNodeNetworkArgs {
            enable_eth_scroll_wire_bridge: false,
            enable_scroll_wire: true,
            ..Default::default()
        },
        database_args: RollupNodeDatabaseArgs {
            rn_db_path: Some(PathBuf::from("sqlite::memory:")),
        },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            auto_start: false,
            block_time: 10,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            allow_empty_blocks: true,
            ..SequencerArgs::default()
        },
        blob_provider_args: BlobProviderArgs { mock: true, ..Default::default() },
        signer_args: Default::default(),
        gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
        rpc_args: RpcArgs::default(),
    };

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Create a sequencer node and an unsynced node.
    let (mut nodes, _databases, _tasks, _) =
        setup_engine(sequencer_node_config.clone(), 1, chain_spec.clone(), false, false)
            .await
            .unwrap();
    let mut sequencer = nodes.pop().unwrap();
    let sequencer_handle = sequencer.inner.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_handle.get_event_listener().await?;
    let sequencer_l1_watcher_tx = sequencer.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();

    let (mut nodes, _databases, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec.clone(), false, false).await.unwrap();
    let mut follower = nodes.pop().unwrap();
    let mut follower_events = follower.inner.rollup_manager_handle.get_event_listener().await?;
    let follower_l1_watcher_tx = follower.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();

    // Connect the nodes together.
    sequencer.connect(&mut follower).await;

    // set both the sequencer and follower L1 watchers to synced
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();
    follower_l1_watcher_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();

    // Initially the sequencer should build 100 blocks with 1 message in each and the follower
    // should follow them
    for i in 0..100 {
        let block_info = BlockInfo { number: i, hash: B256::random() };
        let l1_message = Arc::new(L1Notification::L1Message {
            message: TxL1Message {
                queue_index: i,
                gas_limit: 21000,
                sender: Address::random(),
                to: Address::random(),
                value: U256::from(1),
                input: Default::default(),
            },
            block_info,
            block_timestamp: i * 10,
        });
        let new_block = Arc::new(L1Notification::NewBlock(block_info));
        sequencer_l1_watcher_tx.send(l1_message.clone()).await.unwrap();
        sequencer_l1_watcher_tx.send(new_block.clone()).await.unwrap();
        wait_n_events(
            &mut sequencer_events,
            |e| matches!(e, ChainOrchestratorEvent::NewL1Block(_)),
            1,
        )
        .await;
        follower_l1_watcher_tx.send(l1_message).await.unwrap();
        follower_l1_watcher_tx.send(new_block).await.unwrap();
        wait_n_events(
            &mut follower_events,
            |e| matches!(e, ChainOrchestratorEvent::NewL1Block(_)),
            1,
        )
        .await;

        sequencer_handle.build_block();
        wait_n_events(
            &mut sequencer_events,
            |e| matches!(e, ChainOrchestratorEvent::BlockSequenced(_)),
            1,
        )
        .await;
        wait_n_events(
            &mut follower_events,
            |e| matches!(e, ChainOrchestratorEvent::ChainExtended(_)),
            1,
        )
        .await;
    }

    // send a reorg notification to the sequencer
    sequencer_l1_watcher_tx.send(Arc::new(L1Notification::Reorg(50))).await.unwrap();
    wait_n_events(
        &mut sequencer_events,
        |e| {
            matches!(
                e,
                ChainOrchestratorEvent::L1Reorg {
                    l1_block_number: 50,
                    queue_index: Some(51),
                    l2_head_block_info: _,
                    l2_safe_block_info: _
                }
            )
        },
        1,
    )
    .await;

    sequencer_handle.set_gossip(false).await.unwrap();

    // Have the sequencer build 20 new blocks, containing new L1 messages.
    let mut l1_notifications = vec![];
    for i in 0..20 {
        let block_info = BlockInfo { number: (51 + i), hash: B256::random() };
        let l1_message = Arc::new(L1Notification::L1Message {
            message: TxL1Message {
                queue_index: 51 + i,
                gas_limit: 21000,
                sender: Address::random(),
                to: Address::random(),
                value: U256::from(1),
                input: Default::default(),
            },
            block_info,
            block_timestamp: (51 + i) * 10,
        });
        let new_block = Arc::new(L1Notification::NewBlock(block_info));
        l1_notifications.extend([l1_message.clone(), new_block.clone()]);
        sequencer_l1_watcher_tx.send(l1_message.clone()).await.unwrap();
        sequencer_l1_watcher_tx.send(new_block.clone()).await.unwrap();
        wait_n_events(
            &mut sequencer_events,
            |e| matches!(e, ChainOrchestratorEvent::NewL1Block(_)),
            1,
        )
        .await;

        sequencer_handle.build_block();
        wait_n_events(
            &mut sequencer_events,
            |e| matches!(e, ChainOrchestratorEvent::BlockSequenced(_)),
            1,
        )
        .await;
    }

    // wait two seconds to ensure the timestamp of the new blocks is greater than the old ones
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // now build a final block
    sequencer_handle.set_gossip(true).await.unwrap();
    sequencer_handle.build_block();

    // The follower node should reject the new block as it has a different view of L1 data.
    wait_n_events(
        &mut follower_events,
        |e| matches!(e, ChainOrchestratorEvent::L1MessageMismatch { .. }),
        1,
    )
    .await;

    // Now update the follower node with the new L1 data
    follower_l1_watcher_tx.send(Arc::new(L1Notification::Reorg(50))).await.unwrap();
    for notification in l1_notifications {
        follower_l1_watcher_tx.send(notification).await.unwrap();
    }
    wait_n_events(&mut follower_events, |e| matches!(e, ChainOrchestratorEvent::NewL1Block(_)), 20)
        .await;

    // Now build a new block on the sequencer to trigger the reorg on the follower
    sequencer_handle.build_block();

    // Wait for the follower node to accept the new chain
    wait_n_events(
        &mut follower_events,
        |e| matches!(e, ChainOrchestratorEvent::ChainExtended(_)),
        1,
    )
    .await;

    Ok(())
}

/// Waits for n events to be emitted.
async fn wait_n_events(
    events: &mut EventStream<ChainOrchestratorEvent>,
    mut matches: impl FnMut(ChainOrchestratorEvent) -> bool,
    mut n: u64,
) {
    while let Some(event) = events.next().await {
        if matches(event.clone()) {
            n -= 1;
        }
        if n == 0 {
            break
        }
    }
}
