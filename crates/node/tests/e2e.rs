//! End-to-end tests for the rollup node.

use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{b256, Address, Bytes, Signature, U256};
use futures::{task::noop_waker_ref, FutureExt, StreamExt};
use reth_network::{NetworkConfigBuilder, PeersInfo};
use reth_rpc_api::EthApiServer;
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node::{
    test_utils::{default_test_scroll_rollup_node_config, default_sequencer_test_scroll_rollup_node_config, generate_tx, setup_engine},
    BeaconProviderArgs, DatabaseArgs, EngineDriverArgs, L1ProviderArgs,
    NetworkArgs as ScrollNetworkArgs, ScrollRollupNodeConfig, SequencerArgs,
};
use rollup_node_manager::{RollupManagerEvent, RollupManagerHandle};
use rollup_node_primitives::BatchCommitData;
use rollup_node_providers::BlobSource;
use rollup_node_sequencer::L1MessageInclusionMode;
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_network::{NewBlockWithPeer, SCROLL_MAINNET};
use scroll_wire::{ScrollWireConfig, ScrollWireProtocolHandler};
use std::{
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::Mutex;
use tracing::trace;

#[tokio::test]
async fn can_bridge_l1_messages() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll mainnet with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let node_args = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs::default(),
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
    };
    let (mut nodes, _tasks, _wallet) = setup_engine(node_args, 1, chain_spec, false).await?;
    let node = nodes.pop().unwrap();

    let rnm_handle: RollupManagerHandle = node.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut rnm_events = rnm_handle.get_event_listener().await?;
    let l1_watcher_tx = node.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();

    let l1_message = TxL1Message {
        queue_index: 0,
        gas_limit: 21000,
        sender: Address::random(),
        to: Address::random(),
        value: U256::from(1),
        input: Default::default(),
    };
    l1_watcher_tx
        .send(Arc::new(L1Notification::L1Message {
            message: l1_message.clone(),
            block_number: 0,
            block_timestamp: 1000,
        }))
        .await?;
    if let Some(RollupManagerEvent::L1MessageIndexed(index)) = rnm_events.next().await {
        assert_eq!(index, 0);
    } else {
        panic!("Incorrect index for L1 message");
    };

    rnm_handle.build_block().await;
    if let Some(RollupManagerEvent::BlockSequenced(block)) = rnm_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
        assert_eq!(block.body.transactions[0].as_l1_message().unwrap().inner(), &l1_message,);
    } else {
        panic!("Failed to receive block from rollup node");
    }

    Ok(())
}

#[tokio::test]
async fn can_sequence_and_gossip_blocks() {
    reth_tracing::init_test_tracing();

    // create 2 nodes
    let chain_spec = (*SCROLL_DEV).clone();
    let rollup_manager_args = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
            disable_tx_broadcast: false,
            sequencer_url: None,
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
    };

    let (nodes, _tasks, wallet) =
        setup_engine(rollup_manager_args, 2, chain_spec, false).await.unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    // generate rollup node manager event streams for each node
    let sequencer_rnm_handle = nodes[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await.unwrap();
    let mut follower_events =
        nodes[1].inner.add_ons_handle.rollup_manager_handle.get_event_listener().await.unwrap();

    // inject a transaction into the pool of the first node
    let tx = generate_tx(wallet).await;
    nodes[0].rpc.inject_tx(tx).await.unwrap();
    sequencer_rnm_handle.build_block().await;

    // wait for the sequencer to build a block
    if let Some(RollupManagerEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
    } else {
        panic!("Failed to receive block from rollup node");
    }

    // assert that the follower node has received the block from the peer
    if let Some(RollupManagerEvent::NewBlockReceived(block_with_peer)) =
        follower_events.next().await
    {
        assert_eq!(block_with_peer.block.body.transactions.len(), 1);
    } else {
        panic!("Failed to receive block from rollup node");
    }

    // assert that the block was successfully imported by the follower node
    if let Some(RollupManagerEvent::BlockImported(block)) = follower_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
    } else {
        panic!("Failed to receive block from rollup node");
    }
}



#[tokio::test]
async fn can_forward_tx_to_sequencer() {
    reth_tracing::init_test_tracing();

    // create 2 nodes, first one is a sequencer, second one is a follower
    let chain_spec = (*SCROLL_DEV).clone();
    let rollup_manager_args = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs{
            disable_tx_broadcast: false,
            ..Default::default()
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
    };

    let (nodes, _tasks, wallet) =
        setup_engine(rollup_manager_args, 2, chain_spec, false).await.unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    // generate rollup node manager event streams for each node
    let sequencer_rnm_handle = nodes[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await.unwrap();
    let mut follower_events =
        nodes[1].inner.add_ons_handle.rollup_manager_handle.get_event_listener().await.unwrap();

    // inject a transaction into the pool of the first node
    let tx = generate_tx(wallet).await;
    nodes[1].rpc.inject_tx(tx).await.unwrap();

    // Wait for transaction to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    sequencer_rnm_handle.build_block().await;

    // wait for the sequencer to build a block with the transaction
    if let Some(RollupManagerEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
    } else {
        panic!("Failed to receive block from rollup node");
    }
    

    // assert that the follower node has received the block from the peer
    if let Some(RollupManagerEvent::NewBlockReceived(block_with_peer)) =
        follower_events.next().await
    {
        assert_eq!(block_with_peer.block.body.transactions.len(), 1);
    } else {
        panic!("Failed to receive block from rollup node");
    }

    // assert that the block was successfully imported by the follower node
    if let Some(RollupManagerEvent::BlockImported(block)) = follower_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
    } else {
        panic!("Failed to receive block from rollup node");
    }
}




#[tokio::test]
async fn can_forward_tx_to_sequencer_ai() {
    reth_tracing::init_test_tracing();

    // create sequencer node configuration
    let chain_spec = (*SCROLL_DEV).clone();
    let sequencer_config = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs::default(),
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs::default(),
        signer_args: Default::default(),
    };

    // setup sequencer node
    let (mut sequencer_nodes, _sequencer_tasks, wallet) =
        setup_engine(sequencer_config, 1, chain_spec.clone(), false).await.unwrap();
    let sequencer_node = sequencer_nodes.pop().unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    // get sequencer's RPC URL for follower to connect to
    let sequencer_url = format!("http://localhost:{}", sequencer_node.rpc_url().port().unwrap());

    // create follower node configuration with sequencer_url set
    let follower_config = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs {
            sequencer_url: Some(sequencer_url),
            ..ScrollNetworkArgs::default()
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: false, // follower node, not a sequencer
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs::default(),
        signer_args: Default::default(),
    };

    // setup follower node
    let (mut follower_nodes, _follower_tasks, _) =
        setup_engine(follower_config, 1, chain_spec, false).await.unwrap();
    let follower_node = follower_nodes.pop().unwrap();

    // generate rollup node manager event streams
    let sequencer_rnm_handle = sequencer_node.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await.unwrap();
    let mut follower_events =
        follower_node.inner.add_ons_handle.rollup_manager_handle.get_event_listener().await.unwrap();

    // inject a transaction into the follower node's pool
    let tx = generate_tx(wallet).await;
    follower_node.rpc.inject_tx(tx).await.unwrap();

    // trigger the sequencer to build a block
    sequencer_rnm_handle.build_block().await;

    // wait for the sequencer to build a block containing the forwarded transaction
    if let Some(RollupManagerEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 1, "Sequencer should have included the forwarded transaction");
    } else {
        panic!("Failed to receive block from sequencer node");
    }

    // assert that the follower node has received the block from the sequencer
    if let Some(RollupManagerEvent::NewBlockReceived(block_with_peer)) =
        follower_events.next().await
    {
        assert_eq!(block_with_peer.block.body.transactions.len(), 1, "Follower should receive block with forwarded transaction");
    } else {
        panic!("Failed to receive block from sequencer on follower node");
    }

    // assert that the block was successfully imported by the follower node
    if let Some(RollupManagerEvent::BlockImported(block)) = follower_events.next().await {
        assert_eq!(block.body.transactions.len(), 1, "Follower should import block with forwarded transaction");
    } else {
        panic!("Failed to import block on follower node");
    }
}




/// We test the bridge from the eth-wire protocol to the scroll-wire protocol.
///
/// This test will launch three nodes:
/// - Node 1: The bridge node that will bridge messages from the eth-wire protocol to the
///   scroll-wire protocol.
/// - Node 2: A scroll-wire node that will receive the bridged messages.
/// - Node 3: A standard node that will send messages to the bridge node on the eth-wire protocol.
///
/// The test will send messages from Node 3 to Node 1, which will bridge the messages to Node
/// Node 2 will then receive the messages and verify that they are correct.
#[tokio::test]
async fn can_bridge_blocks() {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Setup the bridge node and a standard node.
    let (mut nodes, tasks, _) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec.clone(), false)
            .await
            .unwrap();
    let mut bridge_node = nodes.pop().unwrap();
    let bridge_peer_id = bridge_node.network.record().id;

    // Instantiate the scroll NetworkManager.
    let network_config = NetworkConfigBuilder::<ScrollNetworkPrimitives>::with_rng_secret_key()
        .disable_discovery()
        .with_unused_listener_port()
        .with_pow()
        .build_with_noop_provider(chain_spec.clone());
    let scroll_wire_config = ScrollWireConfig::new(true);
    let mut scroll_network =
        scroll_network::ScrollNetworkManager::new(network_config, scroll_wire_config).await;
    let scroll_network_handle = scroll_network.handle();

    // Connect the scroll-wire node to the scroll NetworkManager.
    bridge_node.network.add_peer(scroll_network_handle.local_node_record()).await;
    bridge_node.network.next_session_established().await;

    // Create a standard NetworkManager to send blocks to the bridge node.
    let network_config = NetworkConfigBuilder::<ScrollNetworkPrimitives>::with_rng_secret_key()
        .disable_discovery()
        .with_pow()
        .with_unused_listener_port()
        .build_with_noop_provider(chain_spec);

    // Create the standard NetworkManager.
    let network = reth_network::NetworkManager::new(network_config)
        .await
        .expect("Failed to instantiate NetworkManager");
    let network_handle = network.handle().clone();

    // Spawn the standard NetworkManager.
    tasks.executor().spawn(network);

    // Connect the standard NetworkManager to the bridge node.
    bridge_node.network.add_peer(network_handle.local_node_record()).await;
    bridge_node.network.next_session_established().await;

    // Send a block from the standard NetworkManager to the bridge node.
    let mut block_1: reth_scroll_primitives::ScrollBlock =
        serde_json::from_str(include_str!("../assets/block.json")).unwrap();

    // Compute the block hash while masking the extra data which isn't used in block hash
    // computation.
    let extra_data = block_1.extra_data.clone();
    block_1.header.extra_data = Default::default();
    let block_1_hash = block_1.hash_slow();
    block_1.header.extra_data = extra_data.clone();

    let new_block_1 = reth_eth_wire_types::NewBlock { block: block_1, ..Default::default() };

    trace!("Announcing block to eth-wire network");
    network_handle.announce_block(new_block_1, block_1_hash);

    // Assert block received from the bridge node on the scroll wire protocol is correct
    if let Some(scroll_network::NetworkManagerEvent::NewBlock(NewBlockWithPeer {
        peer_id,
        block,
        signature,
    })) = scroll_network.next().await
    {
        assert_eq!(peer_id, bridge_peer_id);
        assert_eq!(block.hash_slow(), block_1_hash);
        assert_eq!(
            TryInto::<Signature>::try_into(extra_data.as_ref().windows(65).last().unwrap())
                .unwrap(),
            signature
        )
    } else {
        panic!("Failed to receive block from scroll-wire network");
    }
}

/// Test that when the rollup node manager is shutdown, it consolidates the most recent batch
/// on startup.
#[tokio::test]
async fn graceful_shutdown_consolidates_most_recent_batch_on_startup() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let chain_spec = (*SCROLL_MAINNET).clone();

    // Launch a node
    let (mut nodes, _tasks, _) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec.clone(), false)
            .await
            .unwrap();
    let node = nodes.pop().unwrap();

    // Instantiate the rollup node manager.
    let mut config = default_test_scroll_rollup_node_config();
    let path = node.inner.config.datadir().db().join("scroll.db?mode=rwc");
    let path = PathBuf::from("sqlite://".to_string() + &*path.to_string_lossy());
    config.database_args.path = Some(path.clone());
    config.beacon_provider_args.url = Some(
        "http://dummy:8545"
            .parse()
            .expect("valid url that will not be used as test batches use calldata"),
    );

    let (_, events) = ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));
    let (mut rnm, handle, l1_notification_tx) = config
        .clone()
        .build(
            node.inner.network.clone(),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
            chain_spec.clone(),
            path.clone(),
        )
        .await?;

    // Request an event stream from the rollup node manager and manually poll rnm to process the
    // event stream request from the handle.
    let mut rnm_events = Box::pin(handle.get_event_listener());
    let mut rnm_events = loop {
        let _ = rnm.poll_unpin(&mut Context::from_waker(noop_waker_ref()));
        if let Poll::Ready(events) =
            rnm_events.poll_unpin(&mut Context::from_waker(noop_waker_ref()))
        {
            break events.unwrap();
        }
    };

    // Extract the L1 notification sender
    let l1_notification_tx = l1_notification_tx.unwrap();

    // Load test batches
    let raw_calldata_0 = read_to_bytes("./tests/testdata/batch_0_calldata.bin")?;
    let batch_0_data = BatchCommitData {
        hash: b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42"),
        index: 1,
        block_number: 18318207,
        block_timestamp: 1696935971,
        calldata: Arc::new(raw_calldata_0),
        blob_versioned_hash: None,
        finalized_block_number: None,
    };
    let raw_calldata_1 = read_to_bytes("./tests/testdata/batch_1_calldata.bin")?;
    let batch_1_data = BatchCommitData {
        hash: b256!("AA8181F04F8E305328A6117FA6BC13FA2093A3C4C990C5281DF95A1CB85CA18F"),
        index: 2,
        block_number: 18318215,
        block_timestamp: 1696936000,
        calldata: Arc::new(raw_calldata_1),
        blob_versioned_hash: None,
        finalized_block_number: None,
    };

    // Send the first batch commit to the rollup node manager.
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_0_data.clone()))).await?;

    // Lets iterate over all blocks expected to be derived from the first batch commit.
    let mut i = 1;
    loop {
        let block_info = loop {
            let event = loop_until_event(&mut rnm, &mut rnm_events).await;
            if let RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome) = event {
                assert!(consolidation_outcome.block_info().block_info.number == i);
                break consolidation_outcome.block_info().block_info;
            }
        };

        if block_info.number == 4 {
            break
        };
        i += 1;
    }

    // Lets finalize the first batch
    l1_notification_tx.send(Arc::new(L1Notification::Finalized(18318208))).await?;

    // Now we send the second batch commit.
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_1_data.clone()))).await?;

    // The second batch commit contains 42 blocks (5-57), lets iterate until the rnm has
    // consolidated up to block 40.
    let mut i = 5;
    let hash = loop {
        let hash = loop {
            let event = loop_until_event(&mut rnm, &mut rnm_events).await;
            if let RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome) = event {
                assert!(consolidation_outcome.block_info().block_info.number == i);
                break consolidation_outcome.block_info().block_info.hash;
            }
        };
        if i == 40 {
            break hash;
        }
        i += 1;
    };

    // Fetch the safe and head block hashes from the EN.
    let rpc = node.rpc.inner.eth_api();
    let safe_block_hash =
        rpc.block_by_number(BlockNumberOrTag::Safe, false).await?.expect("safe block must exist");
    let head_block_hash =
        rpc.block_by_number(BlockNumberOrTag::Latest, false).await?.expect("head block must exist");

    // Assert that the safe block hash is the same as the hash of the last consolidated block.
    assert_eq!(safe_block_hash.header.hash, hash, "Safe block hash does not match expected hash");
    assert_eq!(head_block_hash.header.hash, hash, "Head block hash does not match expected hash");

    // Simulate a shutdown of the rollup node manager by dropping it.
    drop(rnm);
    drop(l1_notification_tx);
    drop(rnm_events);

    // Start the RNM again.
    let (_, events) = ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));
    let (mut rnm, handle, l1_notification_tx) = config
        .clone()
        .build(
            node.inner.network.clone(),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
            chain_spec,
            path.clone(),
        )
        .await?;
    let l1_notification_tx = l1_notification_tx.unwrap();

    // Get a handle to the event stream from the rollup node manager.
    let mut rnm_events = Box::pin(handle.get_event_listener());
    let mut rnm_events = loop {
        let _ = rnm.poll_unpin(&mut Context::from_waker(noop_waker_ref()));
        if let Poll::Ready(events) =
            rnm_events.poll_unpin(&mut Context::from_waker(noop_waker_ref()))
        {
            break events.unwrap();
        }
    };

    // Send the second batch again to mimic the watcher behaviour.
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_0_data.clone()))).await?;
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_1_data.clone()))).await?;

    // Lets fetch the first consolidated block event - this should be the first block of the batch.
    let l2_block = loop {
        let event = loop_until_event(&mut rnm, &mut rnm_events).await;
        if let RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome) = event {
            break consolidation_outcome.block_info().clone();
        }
    };

    // Assert that the consolidated block is the first block of the batch.
    assert_eq!(
        l2_block.block_info.number, 1,
        "Consolidated block number does not match expected number"
    );

    // Lets now iterate over all remaining blocks expected to be derived from the second batch
    // commit.
    for i in 2..=57 {
        loop {
            let event = loop_until_event(&mut rnm, &mut rnm_events).await;
            if let RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome) = event {
                assert!(consolidation_outcome.block_info().block_info.number == i);
                break;
            }
        }
    }

    let safe_block =
        rpc.block_by_number(BlockNumberOrTag::Safe, false).await?.expect("safe block must exist");
    let head_block =
        rpc.block_by_number(BlockNumberOrTag::Latest, false).await?.expect("head block must exist");
    assert_eq!(
        safe_block.header.number, 57,
        "Safe block number should be 57 after all blocks are consolidated"
    );
    assert_eq!(
        head_block.header.number, 57,
        "Head block number should be 57 after all blocks are consolidated"
    );

    Ok(())
}

/// Read the file provided at `path` as a [`Bytes`].
pub fn read_to_bytes<P: AsRef<std::path::Path>>(path: P) -> eyre::Result<Bytes> {
    use std::str::FromStr;
    Ok(Bytes::from_str(&std::fs::read_to_string(path)?)?)
}

async fn loop_until_event(
    rnm: &mut (impl futures::Future<Output = ()> + Unpin),
    rnm_events: &mut (impl futures::Stream<Item = RollupManagerEvent> + Unpin),
) -> RollupManagerEvent {
    loop {
        let _ = rnm.poll_unpin(&mut Context::from_waker(noop_waker_ref()));
        if let Poll::Ready(Some(event)) =
            rnm_events.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()))
        {
            return event;
        }
        tokio::task::yield_now().await;
    }
}
