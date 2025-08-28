//! End-to-end tests for the rollup node.

use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{address, b256, Address, Bytes, B256, U256};
use rollup_node_signer::Signature;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use futures::StreamExt;
use reth_chainspec::EthChainSpec;
use reth_network::{NetworkConfigBuilder, NetworkEventListenerProvider, Peers, PeersInfo};
use reth_network_api::block::EthWireProvider;
use reth_rpc_api::EthApiServer;
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_scroll_primitives::ScrollBlock;
use reth_tokio_util::EventStream;
use rollup_node::{
    constants::SCROLL_GAS_LIMIT,
    test_utils::{
        default_sequencer_test_scroll_rollup_node_config, default_test_scroll_rollup_node_config,
        generate_tx, setup_engine,
    },
    BeaconProviderArgs, ConsensusAlgorithm, ConsensusArgs, DatabaseArgs, EngineDriverArgs,
    GasPriceOracleArgs, L1ProviderArgs, NetworkArgs as ScrollNetworkArgs, RollupNodeContext,
    ScrollRollupNodeConfig, SequencerArgs,
};
use rollup_node_manager::{RollupManagerCommand, RollupManagerEvent};
use rollup_node_primitives::{sig_encode_hash, BatchCommitData, ConsensusUpdate};
use rollup_node_providers::BlobSource;
use rollup_node_sequencer::L1MessageInclusionMode;
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_network::{NewBlockWithPeer, SCROLL_MAINNET};
use scroll_wire::{ScrollWireConfig, ScrollWireProtocolHandler};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    sync::{oneshot, Mutex},
    time,
};
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
    let (mut nodes, _tasks, _wallet) = setup_engine(node_args, 1, chain_spec, false, false).await?;
    let node = nodes.pop().unwrap();

    let rnm_handle = node.inner.add_ons_handle.rollup_manager_handle.clone();
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
            sequencer_url: None,
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
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

    let (nodes, _tasks, wallet) =
        setup_engine(rollup_manager_args, 2, chain_spec, false, false).await.unwrap();
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
    wait_n_events(&mut follower_events, |e| matches!(e, RollupManagerEvent::BlockImported(_)), 1)
        .await;
}

#[tokio::test]
async fn can_penalize_peer_for_invalid_block() {
    reth_tracing::init_test_tracing();

    // create 2 nodes
    let chain_spec = (*SCROLL_DEV).clone();
    let rollup_manager_args = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
            sequencer_url: None,
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
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

    let (nodes, _tasks, _) =
        setup_engine(rollup_manager_args, 2, chain_spec, false, false).await.unwrap();

    let node0_rmn_handle = nodes[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let node0_network_handle = node0_rmn_handle.get_network_handle().await.unwrap();
    let node0_id = node0_network_handle.inner().peer_id();

    let node1_rnm_handle = nodes[1].inner.add_ons_handle.rollup_manager_handle.clone();
    let node1_network_handle = node1_rnm_handle.get_network_handle().await.unwrap();

    // get initial reputation of node0 from pov of node1
    let initial_reputation =
        node1_network_handle.inner().reputation_by_id(*node0_id).await.unwrap().unwrap();
    assert_eq!(initial_reputation, 0);

    // create invalid block
    let block = ScrollBlock::default();

    // send invalid block from node0 to node1. We don't care about the signature here since we use a
    // NoopConsensus in the test.
    node0_network_handle.announce_block(block, Signature::new(U256::from(1), U256::from(1), false));

    eventually(
        Duration::from_secs(5),
        Duration::from_millis(10),
        "Peer0 reputation should be lower after sending invalid block",
        || async {
            // check that the node0 is penalized on node1
            let slashed_reputation =
                node1_network_handle.inner().reputation_by_id(*node0_id).await.unwrap().unwrap();
            slashed_reputation < initial_reputation
        },
    )
    .await;
}

/// Helper function to wait until a predicate is true or a timeout occurs.
pub async fn eventually<F, Fut>(timeout: Duration, tick: Duration, message: &str, mut predicate: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let mut interval = time::interval(tick);
    let start = time::Instant::now();
    loop {
        if predicate().await {
            return;
        }

        assert!(start.elapsed() <= timeout, "Timeout while waiting for condition: {message}");

        interval.tick().await;
    }
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn can_sequence_and_gossip_transactions() {
    reth_tracing::init_test_tracing();

    // create 2 nodes
    let mut sequencer_node_config = default_sequencer_test_scroll_rollup_node_config();
    sequencer_node_config.sequencer_args.block_time = 0;
    let follower_node_config = default_test_scroll_rollup_node_config();

    // Create the chain spec for scroll mainnet with Euclid v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let (mut sequencer_node, _tasks, _) =
        setup_engine(sequencer_node_config, 1, chain_spec.clone(), false, false).await.unwrap();

    let (mut follower_node, _tasks, wallet) =
        setup_engine(follower_node_config, 1, chain_spec, false, false).await.unwrap();

    let wallet = Arc::new(Mutex::new(wallet));

    // Connect the nodes together.
    sequencer_node[0].network.add_peer(follower_node[0].network.record()).await;
    follower_node[0].network.next_session_established().await;
    sequencer_node[0].network.next_session_established().await;

    // generate rollup node manager event streams for each node
    let sequencer_rnm_handle = sequencer_node[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await.unwrap();
    let mut follower_events = follower_node[0]
        .inner
        .add_ons_handle
        .rollup_manager_handle
        .get_event_listener()
        .await
        .unwrap();

    // have the sequencer build an empty block and gossip it to follower
    sequencer_rnm_handle.build_block().await;

    // wait for the sequencer to build a block with no transactions
    if let Some(RollupManagerEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 0);
    } else {
        panic!("Failed to receive block from rollup node");
    }

    // assert that the follower node has received the block from the peer
    wait_n_events(&mut follower_events, |e| matches!(e, RollupManagerEvent::BlockImported(_)), 1)
        .await;

    // inject a transaction into the pool of the follower node
    let tx = generate_tx(wallet).await;
    follower_node[0].rpc.inject_tx(tx).await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    // build block
    sequencer_rnm_handle.build_block().await;

    // wait for the sequencer to build a block with transactions
    wait_n_events(
        &mut sequencer_events,
        |e| {
            if let RollupManagerEvent::BlockSequenced(block) = e {
                assert_eq!(block.header.number, 2);
                assert_eq!(block.body.transactions.len(), 1);
                return true
            }
            false
        },
        1,
    )
    .await;

    // assert that the follower node has received the block from the peer
    wait_n_events(
        &mut follower_events,
        |e| {
            if let RollupManagerEvent::NewBlockReceived(block_with_peer) = e {
                assert_eq!(block_with_peer.block.body.transactions.len(), 1);
                true
            } else {
                false
            }
        },
        1,
    )
    .await;

    // assert that the block was successfully imported by the follower node
    wait_n_events(
        &mut follower_events,
        |e| {
            if let RollupManagerEvent::BlockImported(block) = e {
                assert_eq!(block.body.transactions.len(), 1);
                true
            } else {
                false
            }
        },
        1,
    )
    .await;
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn can_forward_tx_to_sequencer() {
    reth_tracing::init_test_tracing();

    // create 2 nodes
    let mut sequencer_node_config = default_sequencer_test_scroll_rollup_node_config();
    sequencer_node_config.sequencer_args.block_time = 0;
    let mut follower_node_config = default_test_scroll_rollup_node_config();

    // Create the chain spec for scroll mainnet with Euclid v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let (mut sequencer_node, _tasks, _) =
        setup_engine(sequencer_node_config, 1, chain_spec.clone(), false, true).await.unwrap();

    let sequencer_url = format!("http://localhost:{}", sequencer_node[0].rpc_url().port().unwrap());
    follower_node_config.network_args.sequencer_url = Some(sequencer_url);
    let (mut follower_node, _tasks, wallet) =
        setup_engine(follower_node_config, 1, chain_spec, false, true).await.unwrap();

    let wallet = Arc::new(Mutex::new(wallet));

    // Connect the nodes together.
    sequencer_node[0].network.add_peer(follower_node[0].network.record()).await;
    follower_node[0].network.next_session_established().await;
    sequencer_node[0].network.next_session_established().await;

    // generate rollup node manager event streams for each node
    let sequencer_rnm_handle = sequencer_node[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await.unwrap();
    let mut follower_events = follower_node[0]
        .inner
        .add_ons_handle
        .rollup_manager_handle
        .get_event_listener()
        .await
        .unwrap();

    // have the sequencer build an empty block and gossip it to follower
    sequencer_rnm_handle.build_block().await;

    // wait for the sequencer to build a block with no transactions
    if let Some(RollupManagerEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 0);
    } else {
        panic!("Failed to receive block from rollup node");
    }

    // assert that the follower node has received the block from the peer
    wait_n_events(&mut follower_events, |e| matches!(e, RollupManagerEvent::BlockImported(_)), 1)
        .await;

    // inject a transaction into the pool of the follower node
    let tx = generate_tx(wallet).await;
    follower_node[0].rpc.inject_tx(tx).await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    // build block
    sequencer_rnm_handle.build_block().await;

    // wait for the sequencer to build a block with transactions
    wait_n_events(
        &mut sequencer_events,
        |e| {
            if let RollupManagerEvent::BlockSequenced(block) = e {
                assert_eq!(block.header.number, 2);
                assert_eq!(block.body.transactions.len(), 1);
                return true
            }
            false
        },
        1,
    )
    .await;

    // assert that the follower node has received the block from the peer
    wait_n_events(
        &mut follower_events,
        |e| matches!(e, RollupManagerEvent::NewBlockReceived(_)),
        1,
    )
    .await;

    // assert that the block was successfully imported by the follower node
    wait_n_events(
        &mut follower_events,
        |e| {
            if let RollupManagerEvent::BlockImported(block) = e {
                assert_eq!(block.body.transactions.len(), 1);
                true
            } else {
                false
            }
        },
        1,
    )
    .await;
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
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec.clone(), false, false)
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
    let mut scroll_network = scroll_network::ScrollNetworkManager::new(
        network_config,
        scroll_wire_config,
        Default::default(),
    )
    .await;
    let scroll_network_handle = scroll_network.handle();

    // Connect the scroll-wire node to the scroll NetworkManager.
    bridge_node.network.add_peer(scroll_network_handle.local_node_record()).await;
    bridge_node.network.next_session_established().await;

    let genesis_hash = bridge_node.inner.chain_spec().genesis_hash();
    println!("genesis hash: {genesis_hash:?}");

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
    let mut network_events = network_handle.event_listener();

    // Spawn the standard NetworkManager.
    tasks.executor().spawn(network);

    // Connect the standard NetworkManager to the bridge node.
    bridge_node.network.add_peer(network_handle.local_node_record()).await;
    bridge_node.network.next_session_established().await;
    let _ = network_events.next().await;

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
            Signature::from_raw(extra_data.as_ref().windows(65).last().unwrap())
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
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec.clone(), false, false)
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
    let (rnm, handle, l1_notification_tx) = config
        .clone()
        .build(
            RollupNodeContext::new(
                node.inner.network.clone(),
                chain_spec.clone(),
                path.clone(),
                SCROLL_GAS_LIMIT,
            ),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
        )
        .await?;

    // Spawn a task that constantly polls the rnm to make progress.
    let rnm_join_handle = tokio::spawn(async {
        let _ = rnm.await;
    });

    // Request an event stream from the rollup node manager.
    let mut rnm_events = handle.get_event_listener().await?;

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
            if let Some(RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome)) =
                rnm_events.next().await
            {
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
            if let Some(RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome)) =
                rnm_events.next().await
            {
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
    rnm_join_handle.abort();
    drop(l1_notification_tx);
    drop(rnm_events);

    // Start the RNM again.
    let (_, events) = ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));
    let (rnm, handle, l1_notification_tx) = config
        .clone()
        .build(
            RollupNodeContext::new(
                node.inner.network.clone(),
                chain_spec,
                path.clone(),
                SCROLL_GAS_LIMIT,
            ),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
        )
        .await?;
    let l1_notification_tx = l1_notification_tx.unwrap();

    // Spawn a task that constantly polls the rnm to make progress.
    tokio::spawn(async {
        let _ = rnm.await;
    });

    // Request an event stream from the rollup node manager.
    let mut rnm_events = handle.get_event_listener().await?;

    // Send the second batch again to mimic the watcher behaviour.
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_0_data.clone()))).await?;
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_1_data.clone()))).await?;

    // Lets fetch the first consolidated block event - this should be the first block of the batch.
    let l2_block = loop {
        if let Some(RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome)) =
            rnm_events.next().await
        {
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
            if let Some(RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome)) =
                rnm_events.next().await
            {
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

#[tokio::test]
async fn can_handle_batch_revert() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let chain_spec = (*SCROLL_MAINNET).clone();

    // Launch a node
    let (mut nodes, _tasks, _) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec.clone(), false, false)
            .await?;
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
    let (rnm, handle, l1_watcher_tx) = config
        .clone()
        .build(
            RollupNodeContext::new(
                node.inner.network.clone(),
                chain_spec.clone(),
                path.clone(),
                SCROLL_GAS_LIMIT,
            ),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
        )
        .await?;
    let l1_watcher_tx = l1_watcher_tx.unwrap();

    // Spawn a task that constantly polls the rnm to make progress.
    tokio::spawn(async {
        let _ = rnm.await;
    });

    // Request an event stream from the rollup node manager and manually poll rnm to process the
    // event stream request from the handle.
    let mut rnm_events = handle.get_event_listener().await?;

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
    let revert_batch_data = BatchCommitData {
        hash: B256::random(),
        index: 2,
        block_number: 18318220,
        block_timestamp: 1696936500,
        calldata: Arc::new(Default::default()),
        blob_versioned_hash: None,
        finalized_block_number: None,
    };

    // Send the first batch.
    l1_watcher_tx.send(Arc::new(L1Notification::BatchCommit(batch_0_data))).await?;

    // Read the first 4 blocks.
    loop {
        if let Some(RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome)) =
            rnm_events.next().await
        {
            if consolidation_outcome.block_info().block_info.number == 4 {
                break
            }
        }
    }

    // Send the second batch.
    l1_watcher_tx.send(Arc::new(L1Notification::BatchCommit(batch_1_data))).await?;

    // Read the next 42 blocks.
    loop {
        if let Some(RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome)) =
            rnm_events.next().await
        {
            if consolidation_outcome.block_info().block_info.number == 46 {
                break
            }
        }
    }

    let (tx, rx) = oneshot::channel();
    handle.send_command(RollupManagerCommand::Status(tx)).await;

    let status = rx.await?;

    // Assert the forkchoice state is above 4
    assert!(status.forkchoice_state.head_block_info().number > 4);
    assert!(status.forkchoice_state.safe_block_info().number > 4);

    // Send the third batch which should trigger the revert.
    l1_watcher_tx.send(Arc::new(L1Notification::BatchCommit(revert_batch_data))).await?;

    // Wait for the third batch to be proceeded.
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    let (tx, rx) = oneshot::channel();
    handle.send_command(RollupManagerCommand::Status(tx)).await;

    let status = rx.await?;

    // Assert the forkchoice state was reset to 4.
    assert_eq!(status.forkchoice_state.head_block_info().number, 4);
    assert_eq!(status.forkchoice_state.safe_block_info().number, 4);

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn can_handle_reorgs_while_sequencing() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let chain_spec = (*SCROLL_DEV).clone();

    // Launch a node
    let mut config = default_test_scroll_rollup_node_config();
    config.sequencer_args.block_time = 0;
    let (mut nodes, _tasks, _) = setup_engine(config, 1, chain_spec.clone(), false, false).await?;
    let node = nodes.pop().unwrap();

    // Instantiate the rollup node manager.
    let mut config = default_sequencer_test_scroll_rollup_node_config();
    let path = node.inner.config.datadir().db().join("scroll.db?mode=rwc");
    let path = PathBuf::from("sqlite://".to_string() + &*path.to_string_lossy());
    config.database_args.path = Some(path.clone());
    config.beacon_provider_args.url = Some(
        "http://dummy:8545"
            .parse()
            .expect("valid url that will not be used as test batches use calldata"),
    );
    config.engine_driver_args.sync_at_startup = false;
    let (nodes, _tasks, _) = setup_engine(config, 1, chain_spec, false, false).await?;
    let node = nodes.first().unwrap();
    let l1_watcher_tx = node.inner.add_ons_handle.l1_watcher_tx.as_ref().unwrap();
    let mut rnm_events =
        node.inner.add_ons_handle.rollup_manager_handle.get_event_listener().await?;
    let sequencer_rnm_handle = nodes[0].inner.add_ons_handle.rollup_manager_handle.clone();

    // Send an L1 message.
    let message = TxL1Message {
        queue_index: 0,
        gas_limit: 21000,
        to: Default::default(),
        value: Default::default(),
        sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        input: Default::default(),
    };

    // Let the sequencer build 10 blocks before performing the reorg process.
    let mut i = 0;
    loop {
        sequencer_rnm_handle.build_block().await;
        if let Some(RollupManagerEvent::BlockSequenced(_)) = rnm_events.next().await {
            if i == 10 {
                break
            }
            i += 1;
        }
    }

    // Send a L1 message and wait for it to be indexed.
    l1_watcher_tx
        .send(Arc::new(L1Notification::L1Message { message, block_number: 10, block_timestamp: 0 }))
        .await?;
    loop {
        if let Some(RollupManagerEvent::L1MessageIndexed(index)) = rnm_events.next().await {
            assert_eq!(index, 0);
            break
        }
    }
    l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(10))).await?;

    // Wait for block that contains the L1 message.
    sequencer_rnm_handle.build_block().await;
    let l2_reorged_height;
    loop {
        if let Some(RollupManagerEvent::BlockSequenced(block)) = rnm_events.next().await {
            if block.body.transactions.iter().any(|tx| tx.is_l1_message()) {
                l2_reorged_height = block.header.number;
                break
            }
        }
    }

    // Issue and wait for the reorg.
    l1_watcher_tx.send(Arc::new(L1Notification::Reorg(9))).await?;
    loop {
        if let Some(RollupManagerEvent::Reorg(height)) = rnm_events.next().await {
            assert_eq!(height, 9);
            break
        }
    }

    // Get the next sequenced L2 block.
    sequencer_rnm_handle.build_block().await;
    loop {
        if let Some(RollupManagerEvent::BlockSequenced(block)) = rnm_events.next().await {
            assert_eq!(block.number, l2_reorged_height);
            break
        }
    }

    Ok(())
}

#[tokio::test]
async fn can_gossip_over_eth_wire() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Setup the rollup node manager.
    let (mut nodes, _tasks, _) = setup_engine(
        default_sequencer_test_scroll_rollup_node_config(),
        2,
        chain_spec.clone(),
        false,
        false,
    )
    .await
    .unwrap();
    let _sequencer = nodes.pop().unwrap();
    let follower = nodes.pop().unwrap();

    let mut eth_wire_blocks = follower.inner.network.eth_wire_block_listener().await?;

    if let Some(block) = eth_wire_blocks.next().await {
        println!("Received block from eth-wire network: {block:?}");
    } else {
        panic!("Failed to receive block from eth-wire network");
    }

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn signer_rotation() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Create two signers.
    let signer_1 = PrivateKeySigner::random().with_chain_id(Some(chain_spec.chain().id()));
    let signer_1_address = signer_1.address();
    let signer_2 = PrivateKeySigner::random().with_chain_id(Some(chain_spec.chain().id()));
    let signer_2_address = signer_2.address();

    let mut sequencer_1_config = default_sequencer_test_scroll_rollup_node_config();

    sequencer_1_config.test = false;
    sequencer_1_config.consensus_args.algorithm = ConsensusAlgorithm::SystemContract;
    sequencer_1_config.consensus_args.authorized_signer = Some(signer_1_address);
    sequencer_1_config.signer_args.private_key = Some(signer_1);

    let mut sequencer_2_config = default_sequencer_test_scroll_rollup_node_config();
    sequencer_2_config.test = false;
    sequencer_2_config.consensus_args.algorithm = ConsensusAlgorithm::SystemContract;
    sequencer_2_config.consensus_args.authorized_signer = Some(signer_1_address);
    sequencer_2_config.signer_args.private_key = Some(signer_2);

    // Setup two sequencer nodes.
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_1_config, 2, chain_spec.clone(), false, false).await.unwrap();
    let mut sequencer_1 = nodes.pop().unwrap();
    let follower = nodes.pop().unwrap();
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_2_config, 1, chain_spec.clone(), false, false).await.unwrap();
    let mut sequencer_2 = nodes.pop().unwrap();

    // Create an L1
    let follower_l1_notification_tx = follower.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();
    let sequencer_1_l1_notification_tx =
        sequencer_1.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();
    let sequencer_2_l1_notification_tx =
        sequencer_2.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();

    // Create a follower event stream.
    let mut follower_events =
        follower.inner.add_ons_handle.rollup_manager_handle.get_event_listener().await.unwrap();

    // connect the two sequencers
    sequencer_1.connect(&mut sequencer_2).await;

    wait_n_events(
        &mut follower_events,
        |event| {
            if let RollupManagerEvent::NewBlockReceived(block) = event {
                let signature = block.signature;
                let hash = sig_encode_hash(&block.block);
                // Verify that the block is signed by the first sequencer.
                let recovered_address = signature.recover_address_from_prehash(&hash).unwrap();
                recovered_address == signer_1_address
            } else {
                false
            }
        },
        5,
    )
    .await;

    // now update the authorized signer to sequencer 2
    follower_l1_notification_tx
        .send(Arc::new(L1Notification::Consensus(ConsensusUpdate::AuthorizedSigner(
            signer_2_address,
        ))))
        .await?;
    sequencer_1_l1_notification_tx
        .send(Arc::new(L1Notification::Consensus(ConsensusUpdate::AuthorizedSigner(
            signer_2_address,
        ))))
        .await?;
    sequencer_2_l1_notification_tx
        .send(Arc::new(L1Notification::Consensus(ConsensusUpdate::AuthorizedSigner(
            signer_2_address,
        ))))
        .await?;

    wait_n_events(
        &mut follower_events,
        |event| {
            if let RollupManagerEvent::NewBlockReceived(block) = event {
                let signature = block.signature;
                let hash = sig_encode_hash(&block.block);
                let recovered_address = signature.recover_address_from_prehash(&hash).unwrap();
                // Verify that the block is signed by the second sequencer.
                recovered_address == signer_2_address
            } else {
                false
            }
        },
        5,
    )
    .await;

    Ok(())
}

/// Read the file provided at `path` as a [`Bytes`].
pub fn read_to_bytes<P: AsRef<std::path::Path>>(path: P) -> eyre::Result<Bytes> {
    use std::str::FromStr;
    Ok(Bytes::from_str(&std::fs::read_to_string(path)?)?)
}

/// Waits for n events to be emitted.
async fn wait_n_events(
    events: &mut EventStream<RollupManagerEvent>,
    matches: impl Fn(RollupManagerEvent) -> bool,
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
