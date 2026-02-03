//! End-to-end tests for the rollup node.

use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{address, b256, Address, Bytes, Signature, B256, U256};
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use futures::{task::noop_waker_ref, FutureExt, StreamExt};
use reth_chainspec::EthChainSpec;
use reth_network::{NetworkConfigBuilder, NetworkEventListenerProvider, PeersInfo};
use reth_network_api::block::EthWireProvider;
use reth_rpc_api::EthApiServer;
use reth_scroll_chainspec::{ScrollChainSpec, SCROLL_DEV, SCROLL_MAINNET, SCROLL_SEPOLIA};
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_scroll_primitives::ScrollBlock;
use reth_storage_api::BlockReader;
use reth_tasks::shutdown::signal as shutdown_signal;
use reth_tokio_util::EventStream;
use rollup_node::{
    constants::SCROLL_GAS_LIMIT,
    test_utils::{
        default_sequencer_test_scroll_rollup_node_config, default_test_scroll_rollup_node_config,
        generate_tx, setup_engine, EventAssertions, NetworkHelperProvider, ReputationChecks,
        TestFixture,
    },
    RollupNodeAdminApiClient, RollupNodeContext,
};
use rollup_node_chain_orchestrator::{ChainOrchestratorEvent, SyncMode};
use rollup_node_primitives::{sig_encode_hash, BatchCommitData, BlockInfo};
use rollup_node_watcher::L1Notification;
use scroll_db::{test_utils::setup_test_db, L1MessageKey};
use scroll_network::NewBlockWithPeer;
use scroll_wire::{ScrollWireConfig, ScrollWireProtocolHandler};
use std::{
    future::Future,
    path::PathBuf,
    pin::pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{sync::Mutex, time};
use tracing::trace;

#[tokio::test]
async fn can_bridge_l1_messages() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create a sequencer test fixture
    let mut fixture = TestFixture::builder()
        .sequencer()
        .with_l1_message_delay(0)
        .allow_empty_blocks(true)
        .build()
        .await?;

    // Send a notification to set the L1 to synced
    fixture.l1().sync().await?;

    // Create and send an L1 message
    fixture
        .l1()
        .add_message()
        .queue_index(0)
        .gas_limit(21000)
        .sender(Address::random())
        .to(Address::random())
        .value(1u32)
        .at_block(0)
        .send()
        .await?;

    // Wait for the L1 message to be committed
    fixture.expect_event().l1_message_committed().await?;

    // Build a block and expect it to contain the L1 message
    fixture.build_block().expect_l1_message_count(1).build_and_await_block().await?;

    Ok(())
}

#[tokio::test]
async fn can_sequence_and_gossip_blocks() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // create 2 nodes with the new TestFixture API
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .block_time(0)
        .allow_empty_blocks(true)
        .with_eth_scroll_bridge(true)
        .with_scroll_wire(true)
        .payload_building_duration(1000)
        .build()
        .await?;

    // Send L1 synced notification to the sequencer
    fixture.l1().for_node(0).sync().await?;

    // Inject a transaction into the sequencer node
    let tx_hash = fixture.inject_transfer().await?;

    // Build a block and wait for it to be sequenced
    fixture.build_block().expect_tx(tx_hash).expect_tx_count(1).build_and_await_block().await?;

    // Assert that the follower node receives the block from the network
    let received_block = fixture.expect_event_on(1).new_block_received().await?;
    assert_eq!(received_block.body.transactions.len(), 1);

    // Assert that a chain extension is triggered on the follower node
    fixture.expect_event_on(1).chain_extended(1).await?;

    Ok(())
}

#[tokio::test]
async fn can_penalize_peer_for_invalid_block() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create 2 nodes with the TestFixture API
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .block_time(0)
        .allow_empty_blocks(true)
        .with_eth_scroll_bridge(true)
        .with_scroll_wire(true)
        .payload_building_duration(1000)
        .build()
        .await?;

    // Check initial reputation of node 0 from node 1's perspective
    fixture.check_reputation_on(1).of_node(0).await?.equals(0).await?;

    // Create invalid block
    let mut block = ScrollBlock::default();
    block.header.number = 1;
    block.header.parent_hash = fixture.chain_spec.genesis_header.hash();

    // Send invalid block from node0 to node1. We don't care about the signature here since we use a
    // NoopConsensus in the test.
    fixture
        .network_on(0)
        .announce_block(block, Signature::new(U256::from(1), U256::from(1), false))
        .await?;

    // Wait for reputation to decrease
    fixture
        .check_reputation_on(1)
        .of_node(0)
        .await?
        .with_timeout(Duration::from_secs(5))
        .with_poll_interval(Duration::from_millis(10))
        .eventually_less_than(0)
        .await?;

    Ok(())
}

/// Tests that peers are penalized for broadcasting blocks with invalid signatures.
///
/// This test verifies the network's ability to detect and penalize peers that send
/// blocks with either unauthorized or malformed signatures when using the `SystemContract`
/// consensus algorithm.
///
/// The test proceeds in three phases:
/// 1. **Valid signature verification**: Confirms that blocks signed by the authorized signer are
///    accepted and processed normally without peer penalization.
/// 2. **Unauthorized signer detection**: Sends a block signed by an unauthorized signer and
///    verifies that the sending peer's reputation is decreased.
/// 3. **Invalid signature detection**: Sends a block with a malformed signature and verifies
///    further reputation decrease or peer disconnection.
#[tokio::test]
async fn can_penalize_peer_for_invalid_signature() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let chain_spec = (*SCROLL_DEV).clone();

    // Create two signers - one authorized and one unauthorized
    let authorized_signer = PrivateKeySigner::random().with_chain_id(Some(chain_spec.chain().id()));
    let authorized_address = authorized_signer.address();
    let unauthorized_signer =
        PrivateKeySigner::random().with_chain_id(Some(chain_spec.chain().id()));

    // Build fixture with SystemContract consensus
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .with_chain_spec(chain_spec)
        .block_time(0)
        .allow_empty_blocks(true)
        .with_consensus_system_contract(authorized_address)
        .with_signer(authorized_signer.clone())
        .payload_building_duration(1000)
        .build()
        .await?;

    // Set the L1 to synced on the sequencer node
    fixture.l1().for_node(0).sync().await?;
    fixture.expect_event_on(0).l1_synced().await?;

    // === Phase 1: Test valid block with correct signature ===

    // Have the legitimate sequencer build and sign a block
    let block0 = fixture.build_block().expect_tx_count(0).build_and_await_block().await?;

    // Wait for node1 to receive and validate the block with correct signature
    let received_block = fixture.expect_event_on(1).new_block_received().await?;
    assert_eq!(block0.hash_slow(), received_block.hash_slow());

    // Wait for successful import
    fixture.expect_event_on(1).chain_extended(block0.header.number).await?;

    // === Phase 2: Create and send valid block with unauthorized signer signature ===

    // Check initial reputation
    fixture.check_reputation_on(1).of_node(0).await?.equals(0).await?;

    // Create a new block manually (we'll reuse the valid block structure but with wrong signature)
    let mut block1 = block0.clone();
    block1.header.number += 1;
    block1.header.parent_hash = block0.hash_slow();
    block1.header.timestamp += 1;

    // Sign the block with the unauthorized signer
    let block_hash = sig_encode_hash(&block1);
    let unauthorized_signature = unauthorized_signer.sign_hash(&block_hash).await?;

    // Send the block with invalid signature from node0 to node1
    fixture.network_on(0).announce_block(block1.clone(), unauthorized_signature).await?;

    // Node1 should receive and process the invalid block
    fixture
        .expect_event_on(1)
        .timeout(Duration::from_secs(5))
        .extract(|e| {
            if let ChainOrchestratorEvent::NewBlockReceived(block_with_peer) = e {
                if block1.hash_slow() == block_with_peer.block.hash_slow() {
                    // Verify the signature is from the unauthorized signer
                    let hash = sig_encode_hash(&block_with_peer.block);
                    if let Result::Ok(recovered) =
                        block_with_peer.signature.recover_address_from_prehash(&hash)
                    {
                        return Some(recovered == unauthorized_signer.address());
                    }
                }
            }
            None
        })
        .await?;

    // Wait for reputation to decrease
    fixture
        .check_reputation_on(1)
        .of_node(0)
        .await?
        .with_timeout(Duration::from_secs(5))
        .with_poll_interval(Duration::from_millis(100))
        .eventually_less_than(0)
        .await?;

    // === Phase 3: Send valid block with invalid signature ===

    // Get current reputation before sending malformed signature
    let current_reputation = fixture.check_reputation_on(1).of_node(0).await?.get().await?.unwrap();

    let invalid_signature = Signature::new(U256::from(1), U256::from(1), false);

    // Create a new block with the same structure as before but with an invalid signature.
    // We need to make sure the block is different so that it is not filtered.
    block1.header.timestamp += 1;
    fixture.network_on(0).announce_block(block1.clone(), invalid_signature).await?;

    // Wait for the node's 0 reputation to eventually fall.
    fixture
        .check_reputation_on(1)
        .of_node(0)
        .await?
        .with_timeout(Duration::from_secs(5))
        .with_poll_interval(Duration::from_millis(100))
        .eventually_less_than(current_reputation)
        .await?;

    Ok(())
}

/// Tests that peers are penalized for sending duplicate unfinalized blocks via scroll-wire.
#[tokio::test]
async fn can_penalize_peer_for_duplicate_block_via_scroll_wire() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create 2 nodes with scroll-wire enabled
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .block_time(0)
        .allow_empty_blocks(true)
        .payload_building_duration(1000)
        .build()
        .await?;

    // Set the L1 to synced on the sequencer node
    fixture.l1().for_node(0).sync().await?;
    fixture.expect_event_on(0).l1_synced().await?;

    // Build a block
    let block = fixture.build_block().expect_tx_count(0).build_and_await_block().await?;

    // Wait for node1 to receive the block
    fixture.expect_event_on(1).new_block_received().await?;

    // Check initial reputation of node 0 from node 1's perspective
    fixture.check_reputation_on(1).of_node(0).await?.equals(0).await?;

    // Send the same block again (duplicate)
    fixture
        .network_on(0)
        .announce_block(block.clone(), Signature::new(U256::from(1), U256::from(1), false))
        .await?;

    // Wait for reputation to decrease due to duplicate block detection
    fixture
        .check_reputation_on(1)
        .of_node(0)
        .await?
        .with_timeout(Duration::from_secs(5))
        .with_poll_interval(Duration::from_millis(10))
        .eventually_less_than(0)
        .await?;

    Ok(())
}

/// Tests that peers are penalized for sending duplicate unfinalized blocks via eth-wire.
#[tokio::test]
async fn can_penalize_peer_for_duplicate_block_via_eth_wire() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create 2 nodes with scroll-wire disabled
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .block_time(0)
        .allow_empty_blocks(true)
        .with_scroll_wire(false)
        .payload_building_duration(1000)
        .build()
        .await?;

    // Set the L1 to synced on the sequencer node
    fixture.l1().for_node(0).sync().await?;
    fixture.expect_event_on(0).l1_synced().await?;

    // Build a block
    let block = fixture.build_block().expect_tx_count(0).build_and_await_block().await?;

    // Wait for node1 to receive the block
    fixture.expect_event_on(1).new_block_received().await?;

    // Check initial reputation of node 0 from node 1's perspective
    fixture.check_reputation_on(1).of_node(0).await?.equals(0).await?;

    // Send the same block again (duplicate)
    fixture
        .network_on(0)
        .announce_block(block.clone(), Signature::new(U256::from(1), U256::from(1), false))
        .await?;

    // Wait for reputation to decrease due to duplicate block detection
    fixture
        .check_reputation_on(1)
        .of_node(0)
        .await?
        .with_timeout(Duration::from_secs(5))
        .with_poll_interval(Duration::from_millis(10))
        .eventually_less_than(0)
        .await?;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn can_forward_tx_to_sequencer() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // create 2 nodes
    let sequencer_node_config = default_sequencer_test_scroll_rollup_node_config();
    let mut follower_node_config = default_test_scroll_rollup_node_config();

    // Create the chain spec for scroll mainnet with Euclid v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let (mut sequencer_node, _tasks, _) =
        setup_engine(sequencer_node_config, 1, chain_spec.clone(), false, true, None, None)
            .await
            .unwrap();

    let sequencer_url = format!("http://localhost:{}", sequencer_node[0].rpc_url().port().unwrap());
    follower_node_config.network_args.sequencer_url = Some(sequencer_url);
    let (mut follower_node, _tasks, wallet) =
        setup_engine(follower_node_config, 1, chain_spec, false, true, None, None).await.unwrap();

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

    // Send a notification to set the L1 to synced
    let sequencer_l1_watcher_mock = sequencer_node[0]
        .inner
        .add_ons_handle
        .rollup_manager_handle
        .l1_watcher_mock
        .clone()
        .unwrap();
    sequencer_l1_watcher_mock.notification_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();
    sequencer_events.next().await;
    sequencer_events.next().await;

    // have the sequencer build an empty block and gossip it to follower
    sequencer_rnm_handle.build_block();

    // wait for the sequencer to build a block with no transactions
    if let Some(ChainOrchestratorEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 0);
    } else {
        panic!("Failed to receive block from rollup node");
    }

    // assert that the follower node has received the block from the peer
    wait_n_events(
        &mut follower_events,
        |e| matches!(e, ChainOrchestratorEvent::ChainExtended(_)),
        1,
    )
    .await;

    // inject a transaction into the pool of the follower node
    let tx = generate_tx(wallet).await;
    follower_node[0].rpc.inject_tx(tx).await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    // build block
    sequencer_rnm_handle.build_block();

    // wait for the sequencer to build a block with transactions
    wait_n_events(
        &mut sequencer_events,
        |e| {
            if let ChainOrchestratorEvent::BlockSequenced(block) = e {
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
            if let ChainOrchestratorEvent::NewBlockReceived(block_with_peer) = e {
                assert_eq!(block_with_peer.block.body.transactions.len(), 1);
                true
            } else {
                false
            }
        },
        1,
    )
    .await;

    // assert that a chain extension is triggered on the follower node
    wait_n_events(
        &mut follower_events,
        |e| matches!(e, ChainOrchestratorEvent::ChainExtended(_)),
        1,
    )
    .await;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn can_sequence_and_gossip_transactions() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create 2 nodes with the TestFixture API
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .block_time(0)
        .allow_empty_blocks(true)
        .build()
        .await?;

    // Send L1 synced notification to sequencer
    fixture.l1().for_node(0).sync().await?;
    fixture.expect_event_on(0).l1_synced().await?;

    // Have the sequencer build an empty block
    fixture.build_block().expect_tx_count(0).build_and_await_block().await?;

    // Assert that the follower node has received the block
    fixture.expect_event_on(1).chain_extended(1).await?;

    // Inject a transaction into the follower node's pool
    let tx = generate_tx(fixture.wallet.clone()).await;
    fixture.inject_tx_on(1, tx).await?;

    // Wait for transaction propagation
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Build block on sequencer - should include the transaction gossiped from follower
    fixture.build_block().expect_tx_count(1).expect_block_number(2).build_and_await_block().await?;

    // Assert that the follower node has received the block with the transaction
    let received_block = fixture.expect_event_on(1).new_block_received().await?;
    assert_eq!(received_block.body.transactions.len(), 1);

    // Assert that the block was successfully imported by the follower node
    fixture.expect_event_on(1).chain_extended(2).await?;

    Ok(())
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
async fn can_bridge_blocks() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    // Setup the bridge node and a standard node.
    let (mut nodes, tasks, _) = setup_engine(
        default_test_scroll_rollup_node_config(),
        1,
        chain_spec.clone(),
        false,
        false,
        None,
        None,
    )
    .await?;
    let mut bridge_node = nodes.pop().unwrap();
    let bridge_peer_id = bridge_node.network.record().id;
    let bridge_node_l1_watcher_tx =
        bridge_node.inner.add_ons_handle.rollup_manager_handle.l1_watcher_mock.clone().unwrap();

    // Send a notification to set the L1 to synced
    bridge_node_l1_watcher_tx.notification_tx.send(Arc::new(L1Notification::Synced)).await.unwrap();

    // Instantiate the scroll NetworkManager.
    let network_config = NetworkConfigBuilder::<ScrollNetworkPrimitives>::with_rng_secret_key()
        .disable_discovery()
        .with_unused_listener_port()
        .with_pow()
        .build_with_noop_provider(chain_spec.clone());
    let scroll_wire_config = ScrollWireConfig::new(true);
    let (scroll_network, scroll_network_handle) = scroll_network::ScrollNetworkManager::new(
        chain_spec.clone(),
        network_config,
        scroll_wire_config,
        None,
        Default::default(),
        None,
    )
    .await;
    tokio::spawn(scroll_network.run());
    let mut scroll_network_events = scroll_network_handle.event_listener().await;

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
    let mut network_events = network_handle.event_listener();

    // Spawn the standard NetworkManager.
    bridge_node.task_manager.executor().spawn(network);

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
    if let Some(scroll_network::ScrollNetworkManagerEvent::NewBlock(NewBlockWithPeer {
        peer_id,
        block,
        signature,
    })) = scroll_network_events.next().await
    {
        assert_eq!(peer_id, bridge_peer_id);
        assert_eq!(block.hash_slow(), block_1_hash);
        assert_eq!(
            TryInto::<Signature>::try_into(extra_data.as_ref().windows(65).last().unwrap())?,
            signature
        )
    } else {
        panic!("Failed to receive block from scroll-wire network");
    }

    Ok(())
}

/// Test that when the rollup node manager is shutdown, it consolidates the most recent batch
/// on startup.
#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn shutdown_consolidates_most_recent_batch_on_startup() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let chain_spec = (*SCROLL_MAINNET).clone();

    // Launch a node
    let (mut nodes, _tasks, _) = setup_engine(
        default_test_scroll_rollup_node_config(),
        1,
        chain_spec.clone(),
        false,
        false,
        None,
        None,
    )
    .await?;
    let node = nodes.pop().unwrap();

    // Instantiate the rollup node manager.
    let mut config = default_test_scroll_rollup_node_config();
    let path = node.inner.config.datadir().db().join("scroll.db?mode=rwc");
    let path = PathBuf::from("sqlite://".to_string() + &*path.to_string_lossy());
    config.blob_provider_args.beacon_node_urls = Some(vec!["http://dummy:8545"
        .parse()
        .expect("valid url that will not be used as test batches use calldata")]);
    config.hydrate(node.inner.config.clone()).await?;

    let (_, events) = ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));
    let (chain_orchestrator, handle) = config
        .clone()
        .build(
            RollupNodeContext::new(
                node.inner.network.clone(),
                chain_spec.clone(),
                path.clone(),
                SCROLL_GAS_LIMIT,
                node.inner.task_executor.clone(),
            ),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
        )
        .await?;

    // Spawn a task that constantly polls the rnm to make progress.
    let (signal, shutdown) = shutdown_signal();
    tokio::spawn(async {
        let (_signal, inner) = shutdown_signal();
        let chain_orchestrator = chain_orchestrator.run_until_shutdown(inner);
        tokio::select! {
            biased;

            _ = shutdown => {},
            _ = chain_orchestrator => {},
        }
    });

    // Request an event stream from the rollup node manager.
    let mut rnm_events = handle.get_event_listener().await?;

    // Extract the L1 notification sender
    let l1_notification_tx = handle.l1_watcher_mock.unwrap();

    // Load test batches
    let block_0_info = BlockInfo { number: 18318207, hash: B256::random() };
    let raw_calldata_0 = read_to_bytes("./tests/testdata/batch_0_calldata.bin")?;
    let batch_0_data = BatchCommitData {
        hash: b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42"),
        index: 1,
        block_number: 18318207,
        block_timestamp: 1696935971,
        calldata: Arc::new(raw_calldata_0),
        blob_versioned_hash: None,
        finalized_block_number: None,
        reverted_block_number: None,
    };
    let block_1_info = BlockInfo { number: 18318215, hash: B256::random() };
    let raw_calldata_1 = read_to_bytes("./tests/testdata/batch_1_calldata.bin")?;
    let batch_1_data = BatchCommitData {
        hash: b256!("AA8181F04F8E305328A6117FA6BC13FA2093A3C4C990C5281DF95A1CB85CA18F"),
        index: 2,
        block_number: 18318215,
        block_timestamp: 1696936000,
        calldata: Arc::new(raw_calldata_1),
        blob_versioned_hash: None,
        finalized_block_number: None,
        reverted_block_number: None,
    };

    // Send the first batch commit to the rollup node manager and finalize it.
    l1_notification_tx
        .notification_tx
        .send(Arc::new(L1Notification::BatchCommit {
            block_info: block_0_info,
            data: batch_0_data.clone(),
        }))
        .await?;
    l1_notification_tx
        .notification_tx
        .send(Arc::new(L1Notification::BatchFinalization {
            hash: batch_0_data.hash,
            index: batch_0_data.index,
            block_info: block_0_info,
        }))
        .await?;

    // Lets finalize the first batch
    l1_notification_tx
        .notification_tx
        .send(Arc::new(L1Notification::Finalized(block_0_info.number)))
        .await?;

    // Lets iterate over all blocks expected to be derived from the first batch commit.
    let consolidation_outcome = loop {
        let event = rnm_events.next().await;
        println!("Received event: {:?}", event);
        if let Some(ChainOrchestratorEvent::BatchConsolidated(consolidation_outcome)) = event {
            break consolidation_outcome;
        }
    };
    assert_eq!(consolidation_outcome.blocks.len(), 4, "Expected 4 blocks to be consolidated");

    // Now we send the second batch commit and finalize it.
    l1_notification_tx
        .notification_tx
        .send(Arc::new(L1Notification::BatchCommit {
            block_info: block_1_info,
            data: batch_1_data.clone(),
        }))
        .await?;
    l1_notification_tx
        .notification_tx
        .send(Arc::new(L1Notification::BatchFinalization {
            hash: batch_1_data.hash,
            index: batch_1_data.index,
            block_info: block_1_info,
        }))
        .await?;

    // Lets finalize the second batch.
    l1_notification_tx
        .notification_tx
        .send(Arc::new(L1Notification::Finalized(block_1_info.number)))
        .await?;

    // The second batch commit contains 42 blocks (5-57), lets iterate until the rnm has
    // consolidated up to block 40.
    let mut i = 5;
    let hash = loop {
        let hash = loop {
            if let Some(ChainOrchestratorEvent::BlockConsolidated(consolidation_outcome)) =
                rnm_events.next().await
            {
                assert_eq!(consolidation_outcome.block_info().block_info.number, i);
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
    assert_eq!(
        safe_block_hash.header.hash, hash,
        "Safe block hash does not match expected
    hash"
    );
    assert_eq!(
        head_block_hash.header.hash, hash,
        "Head block hash does not match
    expected hash"
    );

    // Simulate a shutdown of the rollup node manager by dropping it.
    signal.fire();
    drop(l1_notification_tx);
    drop(rnm_events);

    // Start the RNM again.
    let (_, events) = ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));
    let (chain_orchestrator, handle) = config
        .clone()
        .build(
            RollupNodeContext::new(
                node.inner.network.clone(),
                chain_spec,
                path.clone(),
                SCROLL_GAS_LIMIT,
                node.inner.task_executor.clone(),
            ),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
        )
        .await?;
    let l1_notification_tx = handle.l1_watcher_mock.clone().unwrap();

    // Spawn a task that constantly polls the rnm to make progress.
    let (_signal, shutdown) = shutdown_signal();
    tokio::spawn(async {
        let (_signal, inner) = shutdown_signal();
        let chain_orchestrator = chain_orchestrator.run_until_shutdown(inner);
        tokio::select! {
            biased;

            _ = shutdown => {},
            _ = chain_orchestrator => {},
        }
    });

    // Request an event stream from the rollup node manager.
    let mut rnm_events = handle.get_event_listener().await?;

    // Send the second batch again to mimic the watcher behaviour.
    let block_1_info = BlockInfo { number: 18318215, hash: B256::random() };
    l1_notification_tx
        .notification_tx
        .send(Arc::new(L1Notification::Finalized(block_1_info.number)))
        .await?;

    // Lets fetch the first consolidated block event - this should be the first block of the batch.
    let l2_block = loop {
        if let Some(ChainOrchestratorEvent::BlockConsolidated(consolidation_outcome)) =
            rnm_events.next().await
        {
            break consolidation_outcome.block_info().clone();
        }
    };

    // One issue #273 is completed, we will again have safe blocks != finalized blocks, and this
    // should be changed to 1. Assert that the consolidated block is the first block that was not
    // previously processed of the batch.
    assert_eq!(
        l2_block.block_info.number, 41,
        "Consolidated block number does not match expected number"
    );

    // Lets now iterate over all remaining blocks expected to be derived from the second batch
    // commit.
    for i in 42..=57 {
        loop {
            if let Some(ChainOrchestratorEvent::BlockConsolidated(consolidation_outcome)) =
                rnm_events.next().await
            {
                assert!(consolidation_outcome.block_info().block_info.number == i);
                break;
            }
        }
    }

    let finalized_block = rpc
        .block_by_number(BlockNumberOrTag::Finalized, false)
        .await?
        .expect("finalized block must exist");
    let safe_block =
        rpc.block_by_number(BlockNumberOrTag::Safe, false).await?.expect("safe block must exist");
    let head_block =
        rpc.block_by_number(BlockNumberOrTag::Latest, false).await?.expect("head block must exist");
    assert_eq!(
        finalized_block.header.number, 57,
        "Finalized block number should be 57 after all blocks are consolidated"
    );
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

/// Test that when the rollup node manager is shutdown, it restarts with the head set to the latest
/// signed block stored in database.
#[tokio::test]
async fn graceful_shutdown_sets_fcs_to_latest_signed_block_in_db_on_start_up() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let chain_spec = (*SCROLL_DEV).clone();

    // Create a config with a random signer.
    let mut config = default_sequencer_test_scroll_rollup_node_config();
    config.signer_args.private_key = Some(PrivateKeySigner::random());

    // Launch a node
    let (mut nodes, _tasks, _) =
        setup_engine(config.clone(), 1, chain_spec.clone(), false, false, None, None).await?;
    let node = nodes.pop().unwrap();

    // Instantiate the rollup node manager.
    let test_db = setup_test_db().await;
    let path = test_db.tmp_dir().expect("Database started with temp dir").path().join("test.db");
    config.blob_provider_args.beacon_node_urls = Some(vec!["http://dummy:8545"
        .parse()
        .expect("valid url that will not be used as test batches use calldata")]);
    config.hydrate(node.inner.config.clone()).await?;

    let (_, events) = ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));
    let (rnm, handle) = config
        .clone()
        .build(
            RollupNodeContext::new(
                node.inner.network.clone(),
                chain_spec.clone(),
                path.clone(),
                SCROLL_GAS_LIMIT,
                node.inner.task_executor.clone(),
            ),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
        )
        .await?;
    let (_signal, shutdown) = shutdown_signal();
    let mut rnm = Box::pin(rnm.run_until_shutdown(shutdown));
    let l1_watcher_mock = handle.l1_watcher_mock.clone().unwrap();

    // Poll the rnm until we get an event stream listener.
    let mut rnm_events_fut = pin!(handle.get_event_listener());
    let mut rnm_events = loop {
        let _ = rnm.poll_unpin(&mut Context::from_waker(noop_waker_ref()));
        if let Poll::Ready(Result::Ok(events)) =
            rnm_events_fut.as_mut().poll(&mut Context::from_waker(noop_waker_ref()))
        {
            break events;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    // Poll the rnm until we receive the consolidate event
    l1_watcher_mock.notification_tx.send(Arc::new(L1Notification::Synced)).await?;
    loop {
        let _ = rnm.poll_unpin(&mut Context::from_waker(noop_waker_ref()));
        if let Poll::Ready(Some(ChainOrchestratorEvent::ChainConsolidated { from: _, to: _ })) =
            rnm_events.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()))
        {
            break
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Wait for the EN to be synced to block 10.
    let execution_node_provider = &node.inner.provider;
    loop {
        handle.build_block();
        let block_number = loop {
            let _ = rnm.poll_unpin(&mut Context::from_waker(noop_waker_ref()));
            if let Poll::Ready(Some(ChainOrchestratorEvent::SignedBlock { block, signature: _ })) =
                rnm_events.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()))
            {
                break block.header.number
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        if block_number == 10 {
            break
        }
    }

    // Get the block info for block 10.
    let db_head_block_info = execution_node_provider
        .block(10u64.into())?
        .map(|b| BlockInfo { number: b.number, hash: b.hash_slow() })
        .expect("block exists");

    // At this point, we have the EN synced to a block > 10 and the RNM has sequenced one additional
    // block, validating it with the EN, but not updating the last sequenced block in the DB.
    // Simulate a shutdown of the rollup node manager by dropping it.
    drop(rnm_events);
    drop(rnm);

    // Start the RNM again.
    let (_, events) = ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));
    let (rnm, handle) = config
        .clone()
        .build(
            RollupNodeContext::new(
                node.inner.network.clone(),
                chain_spec,
                path.clone(),
                SCROLL_GAS_LIMIT,
                node.inner.task_executor.clone(),
            ),
            events,
            node.inner.add_ons_handle.rpc_handle.rpc_server_handles.clone(),
        )
        .await?;

    // Launch the rnm in a task.
    tokio::spawn(async {
        let (_signal, inner) = shutdown_signal();
        rnm.run_until_shutdown(inner).await;
    });

    // Check the fcs.
    let status = handle.status().await?;

    // The fcs should be set to the database head.
    assert_eq!(status.l2.fcs.head_block_info(), &db_head_block_info);

    Ok(())
}

#[tokio::test]
async fn can_revert_to_l1_block() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create a follower test fixture using SCROLL_MAINNET chain spec
    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_MAINNET.clone())
        .with_memory_db()
        .build()
        .await?;

    // Load test batches
    let batch_0_block_info = BlockInfo { number: 18318207, hash: B256::random() };
    let raw_calldata_0 = read_to_bytes("./tests/testdata/batch_0_calldata.bin")?;
    let batch_0_hash = b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42");

    let batch_1_block_info = BlockInfo { number: 18318215, hash: B256::random() };
    let raw_calldata_1 = read_to_bytes("./tests/testdata/batch_1_calldata.bin")?;
    let batch_1_hash = b256!("AA8181F04F8E305328A6117FA6BC13FA2093A3C4C990C5281DF95A1CB85CA18F");

    // Send a Synced notification to the chain orchestrator
    fixture.l1().sync().await?;

    // Send the first batch
    let _ = fixture
        .l1()
        .commit_batch()
        .at_block(batch_0_block_info)
        .hash(batch_0_hash)
        .index(1)
        .calldata(raw_calldata_0)
        .send()
        .await?;

    // Wait for the batch consolidated event
    fixture.expect_event().batch_consolidated().await?;

    // Send the second batch
    let _ = fixture
        .l1()
        .commit_batch()
        .at_block(batch_1_block_info)
        .hash(batch_1_hash)
        .index(2)
        .calldata(raw_calldata_1.clone())
        .send()
        .await?;

    // Wait for the second batch to be consolidated
    fixture.expect_event().batch_consolidated().await?;

    // Get the node status
    let status = fixture.get_status(0).await?;

    // Validate the safe block number is 57
    assert_eq!(status.l2.fcs.safe_block_info().number, 57);

    // Now send a revert to L1 block 18318210
    fixture.follower(0).rollup_manager_handle.revert_to_l1_block(18318210).await?;

    // Wait for the chain to be unwound
    fixture.expect_event().revert_to_l1_block().await?;

    // Now have the L1 watcher mock handle the command to rewind the L1 head.
    fixture
        .follower(0)
        .rollup_manager_handle
        .l1_watcher_mock
        .as_mut()
        .unwrap()
        .handle_command()
        .await;

    // Get the node status
    let status = fixture.get_status(0).await?;

    // Assert that the safe block number is now 4
    assert_eq!(status.l2.fcs.safe_block_info().number, 4);

    // Assert that the L1 status is now syncing
    assert_eq!(status.l1.status, SyncMode::Syncing);

    // Now send a Synced notification to the chain orchestrator to resume eager processing
    fixture.l1().sync().await?;

    // Send the second batch again
    let _ = fixture
        .l1()
        .commit_batch()
        .at_block(batch_1_block_info)
        .hash(batch_1_hash)
        .index(2)
        .calldata(raw_calldata_1)
        .send()
        .await?;

    // Wait for the second batch to be consolidated
    fixture.expect_event().batch_consolidated().await?;

    // Get the node status
    let status = fixture.get_status(0).await?;

    // Validate the safe block number is 57
    assert_eq!(status.l2.fcs.safe_block_info().number, 57);
    assert_eq!(status.l1.status, SyncMode::Synced);

    Ok(())
}

#[tokio::test]
async fn consolidates_committed_batches_after_chain_consolidation() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create a follower test fixture using SCROLL_MAINNET chain spec
    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_MAINNET.clone())
        .with_memory_db()
        .build()
        .await?;

    // Load test batches
    let batch_0_block_info = BlockInfo { number: 18318207, hash: B256::random() };
    let raw_calldata_0 = read_to_bytes("./tests/testdata/batch_0_calldata.bin")?;
    let batch_0_hash = b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42");

    let batch_0_finalization_block_info = BlockInfo { number: 18318210, hash: B256::random() };
    let batch_1_block_info = BlockInfo { number: 18318215, hash: B256::random() };
    let raw_calldata_1 = read_to_bytes("./tests/testdata/batch_1_calldata.bin")?;
    let batch_1_hash = b256!("AA8181F04F8E305328A6117FA6BC13FA2093A3C4C990C5281DF95A1CB85CA18F");

    let batch_1_finalization_block_info = BlockInfo { number: 18318220, hash: B256::random() };

    // Send the first batch
    let (_, batch_0_index) = fixture
        .l1()
        .commit_batch()
        .at_block(batch_0_block_info)
        .hash(batch_0_hash)
        .index(1)
        .calldata(raw_calldata_0)
        .send()
        .await?;

    // Send a batch finalization for the first batch
    fixture
        .l1()
        .finalize_batch()
        .hash(batch_0_hash)
        .index(batch_0_index)
        .at_block(batch_0_finalization_block_info)
        .send()
        .await?;

    // Send the L1 block finalized notification
    fixture.l1().finalize_l1_block(batch_0_finalization_block_info.number).await?;

    // Wait for batch consolidated event
    fixture.expect_event().batch_consolidated().await?;

    // Send the second batch
    let (_, batch_1_index) = fixture
        .l1()
        .commit_batch()
        .at_block(batch_1_block_info)
        .hash(batch_1_hash)
        .index(2)
        .calldata(raw_calldata_1)
        .send()
        .await?;

    // Send the Synced notification to the chain orchestrator
    fixture.l1().sync().await?;

    // Wait for the second batch to be consolidated
    fixture.expect_event().batch_consolidated().await?;

    let status = fixture.get_sequencer_status().await?;

    assert_eq!(status.l2.fcs.safe_block_info().number, 57);
    assert_eq!(status.l2.fcs.finalized_block_info().number, 4);

    // Now send the batch finalization event for the second batch and finalize the L1 block
    fixture
        .l1()
        .finalize_batch()
        .hash(batch_1_hash)
        .index(batch_1_index)
        .at_block(batch_1_finalization_block_info)
        .send()
        .await?;

    fixture.l1().finalize_l1_block(batch_1_finalization_block_info.number).await?;

    // Wait for L1 block finalized event
    fixture.expect_event().l1_block_finalized().await?;

    let status = fixture.get_sequencer_status().await?;

    assert_eq!(status.l2.fcs.safe_block_info().number, 57);
    assert_eq!(status.l2.fcs.finalized_block_info().number, 57);

    Ok(())
}

#[tokio::test]
async fn can_handle_batch_revert_with_reorg() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create a follower test fixture using SCROLL_MAINNET chain spec
    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_MAINNET.clone())
        .with_memory_db()
        .build()
        .await?;

    // Send a Synced notification to the chain orchestrator
    fixture.l1().sync().await?;

    // Load test batches
    let batch_0_block_info = BlockInfo { number: 18318207, hash: B256::random() };
    let raw_calldata_0 = read_to_bytes("./tests/testdata/batch_0_calldata.bin")?;
    let batch_0_hash = b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42");

    let batch_1_block_info = BlockInfo { number: 18318215, hash: B256::random() };
    let raw_calldata_1 = read_to_bytes("./tests/testdata/batch_1_calldata.bin")?;
    let batch_1_hash = b256!("AA8181F04F8E305328A6117FA6BC13FA2093A3C4C990C5281DF95A1CB85CA18F");

    let batch_1_revert_block_info = BlockInfo { number: 18318216, hash: B256::random() };

    // Send the first batch
    fixture
        .l1()
        .commit_batch()
        .at_block(batch_0_block_info)
        .hash(batch_0_hash)
        .index(1)
        .at_block(batch_0_block_info)
        .block_timestamp(1696935971)
        .calldata(raw_calldata_0)
        .send()
        .await?;

    // Wait for block 4 to be consolidated
    fixture.expect_event().block_consolidated(4).await?;

    // Send the second batch
    let (_, batch_1_index) = fixture
        .l1()
        .commit_batch()
        .at_block(batch_1_block_info)
        .hash(batch_1_hash)
        .index(2)
        .at_block(batch_1_block_info)
        .block_timestamp(1696936000)
        .calldata(raw_calldata_1)
        .send()
        .await?;

    // Wait for block 57 to be consolidated (after processing batch 1)
    fixture.expect_event().block_consolidated(57).await?;

    let status = fixture.get_sequencer_status().await?;
    assert!(status.l2.fcs.head_block_info().number > 4);
    assert!(status.l2.fcs.safe_block_info().number > 4);

    // Send the revert for the second batch
    fixture
        .l1()
        .revert_batch()
        .hash(batch_1_hash)
        .index(batch_1_index)
        .at_block(batch_1_revert_block_info)
        .send()
        .await?;

    // Wait for batch reverted event
    fixture.expect_event().batch_reverted().await?;

    let status = fixture.get_sequencer_status().await?;

    // Assert the forkchoice state was reset to 4
    assert_eq!(status.l2.fcs.head_block_info().number, 57);
    assert_eq!(status.l2.fcs.safe_block_info().number, 4);

    // Now let's reorg the L1 such that the batch revert should be reorged out
    fixture.l1().reorg_to(18318215).await?;

    // Wait for L1 reorg event
    fixture.expect_event().l1_reorg().await?;

    let status = fixture.get_sequencer_status().await?;

    // Assert the forkchoice state safe block was reset to 57
    assert_eq!(status.l2.fcs.safe_block_info().number, 57);

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn can_handle_l1_message_reorg() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    color_eyre::install()?;

    // Launch 2 nodes: node0=sequencer and node1=follower.
    let mut fixture = TestFixture::builder().sequencer().followers(1).block_time(0).build().await?;

    // Set L1 synced on both the sequencer and follower nodes.
    fixture.l1().sync().await?;
    fixture.expect_event_on_all_nodes().l1_synced().await?;

    // Let the sequencer build 10 blocks before performing the reorg process.
    let mut reorg_block = None;
    for i in 1..=10 {
        let b = fixture.build_block().expect_block_number(i).build_and_await_block().await?;
        tracing::info!(target: "scroll::test", block_number = ?b.header.number, block_hash = ?b.header.hash_slow(), "Sequenced block");
        reorg_block = Some(b);
    }

    // Assert that the follower node has received all 10 blocks from the sequencer node.
    fixture
        .expect_event_on(1)
        .where_n_events(10, |e| matches!(e, ChainOrchestratorEvent::ChainExtended(_)))
        .await?;

    // Send the L1 message to the nodes.
    fixture
        .l1()
        .add_message()
        .at_block(10)
        .sender(address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"))
        .send()
        .await?;
    fixture.l1().new_block(10).await?;

    // Expect the events to reach all nodes.
    fixture.expect_event_on_all_nodes().l1_message_committed().await?;
    fixture.expect_event_on_all_nodes().new_l1_block().await?;

    // Build block that contains the L1 message.
    let block11_before_reorg = fixture
        .build_block()
        .expect_block_number(11)
        .expect_l1_message_count(1)
        .build_and_await_block()
        .await?;

    for i in 12..=15 {
        fixture.build_block().expect_block_number(i).build_and_await_block().await?;
    }

    // Assert that the follower node has received the latest block from the sequencer node.
    fixture
        .expect_event_on(1)
        .where_n_events(5, |e| matches!(e, ChainOrchestratorEvent::ChainExtended(_)))
        .await?;

    // Assert both nodes are at block 15.
    let sequencer_latest_block = fixture.get_sequencer_block().await?;
    let follower_latest_block = fixture.get_block(1).await?;
    assert_eq!(sequencer_latest_block.header.number, 15);
    assert_eq!(sequencer_latest_block.header.hash_slow(), follower_latest_block.header.hash_slow());

    // Issue and wait for the reorg.
    fixture.l1().reorg_to(9).await?;
    fixture
        .expect_event_on_all_nodes()
        .where_event(|e| {
            matches!(
                e,
                ChainOrchestratorEvent::L1Reorg {
                    l1_block_number: 9,
                    queue_index: Some(0),
                    l2_head_block_info: b,
                    l2_safe_block_info: None,
                }
                if *b == reorg_block.as_ref().map(|b| b.into())
            )
        })
        .await?;

    // Wait for block to handle the reorg
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Assert both nodes are at block 10.
    assert_eq!(fixture.get_sequencer_block().await?.header.number, 10);
    assert_eq!(fixture.get_block(1).await?.header.number, 10);

    // Since the L1 reorg reverted the L1 message included in block 11, the sequencer
    // should produce a new block at height 11.
    fixture.build_block().expect_block_number(11).build_and_await_block().await?;

    // Assert that the follower node has received the new block from the sequencer node.
    fixture.expect_event_on(1).chain_extended(11).await?;

    // Assert both nodes are at block 11.
    let sequencer_block11 = fixture.get_sequencer_block().await?;
    assert_eq!(sequencer_block11.header.number, 11);
    assert_eq!(fixture.get_block(1).await?.header.number, 11);

    // Assert that block 11 has a different hash after the reorg.
    assert_ne!(block11_before_reorg.hash_slow(), sequencer_block11.header.hash_slow());

    Ok(())
}

/// Test that when L2 block reorg happens due to an L1 reorg, the transactions that were reverted
/// are requeued.
#[tokio::test]
async fn requeues_transactions_after_l1_reorg() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut sequencer =
        TestFixture::builder().sequencer().auto_start(false).block_time(0).build().await?;

    // Set the l1 as being synced.
    sequencer.l1().sync().await?;

    // Let the sequencer build 10 blocks.
    for i in 1..=10 {
        let b = sequencer.build_block().expect_block_number(i).build_and_await_block().await?;
        tracing::info!(target: "scroll::test", block_number = ?b.header.number, block_hash = ?b.header.hash_slow(), "Sequenced block");
    }

    // Send a L1 message and wait for it to be indexed.
    sequencer.l1().add_message().at_block(2).send().await?;
    sequencer.expect_event().l1_message_committed().await?;

    // Send the L1 block which finalizes the L1 message.
    sequencer.l1().new_block(2).await?;
    sequencer.expect_event().new_l1_block().await?;

    // Build a L2 block with L1 message, so we can revert it later.
    sequencer.build_block().build_and_await_block().await?;

    // Inject a user transaction and force the sequencer to include it in the next block
    let hash = sequencer.inject_transfer().await?;
    sequencer.build_block().expect_tx(hash).expect_tx_count(1).build_and_await_block().await?;

    // Trigger an L1 reorg that reverts the block containing the transaction
    sequencer.l1().reorg_to(1).await?;
    sequencer.expect_event().l1_reorg().await?;

    // Build the next block – the reverted transaction should have been requeued
    sequencer.build_block().expect_tx(hash).expect_tx_count(1).build_and_await_block().await?;

    Ok(())
}

/// Test that when the FCS head is reset to an earlier block via `UpdateFcsHead`,
/// the transactions from reverted blocks are requeued into the tx pool and can
/// be included again.
#[tokio::test]
async fn requeues_transactions_after_update_fcs_head() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut sequencer =
        TestFixture::builder().sequencer().auto_start(false).block_time(0).build().await?;

    // Set the l1 as being synced.
    sequencer.l1().sync().await?;

    // Build a few blocks and remember block #4 as the future reset target.
    let mut target_head: Option<BlockInfo> = None;
    for i in 1..=4 {
        let b =
            sequencer.build_block().expect_block_number(i as u64).build_and_await_block().await?;
        if i == 4 {
            target_head = Some(BlockInfo { number: b.header.number, hash: b.header.hash_slow() });
        }
    }

    // Inject a user transaction and include it in block 5.
    let hash = sequencer.inject_transfer().await?;
    sequencer
        .build_block()
        .expect_block_number(5)
        .expect_tx(hash)
        .expect_tx_count(1)
        .build_and_await_block()
        .await?;

    // Reset FCS head back to block 4; this should collect block 5's txs and requeue them.
    let head = target_head.expect("target head exists");
    sequencer
        .sequencer()
        .rollup_manager_handle
        .update_fcs_head(head)
        .await
        .expect("update_fcs_head should succeed");

    // Build the next block – the reverted transaction should have been requeued and included.
    sequencer
        .build_block()
        .expect_block_number(5)
        .expect_tx(hash)
        .expect_tx_count(1)
        .build_and_await_block()
        .await?;

    Ok(())
}

/// Tests that a sequencer and follower node can produce blocks using a custom local genesis
/// configuration and properly propagate them between nodes.
#[tokio::test]
async fn test_custom_genesis_block_production_and_propagation() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    color_eyre::install()?;

    // Create a custom genesis config.
    // In a real scenario, this JSON would be loaded from a file using --chain flag.
    let custom_genesis_json = r#"{
        "config": {
            "chainId": 999999,
            "homesteadBlock": 0,
            "eip150Block": 0,
            "eip155Block": 0,
            "eip158Block": 0,
            "byzantiumBlock": 0,
            "constantinopleBlock": 0,
            "petersburgBlock": 0,
            "istanbulBlock": 0,
            "berlinBlock": 0,
            "londonBlock": 0,
            "archimedesBlock": 0,
            "shanghaiBlock": 0,
            "bernoulliBlock": 0,
            "curieBlock": 0,
            "darwinTime": 0,
            "darwinV2Time": 0,
            "euclidTime": 0,
            "euclidV2Time": 0,
            "feynmanTime": 0,
            "scroll": {
                "feeVaultAddress": "0x5300000000000000000000000000000000000005",
                "maxTxPayloadBytesPerBlock": 122880,
                "l1Config": {
                    "l1ChainId": 1,
                    "l1MessageQueueAddress": "0x0d7E906BD9cAFa154b048cFa766Cc1E54E39AF9B",
                    "l1MessageQueueV2Address": "0x000000000000000000000000000000000003dead",
                    "scrollChainAddress": "0x000000000000000000000000000000000003dead",
                    "l2SystemConfigAddress": "0x0000000000000000000000000000000dddd3dead",
                    "systemContractAddress": "0x110000000000000000000000000000000003dead",
                    "numL1MessagesPerBlock": 10,
                    "startL1Block": 12345
                }
            }
        },
        "nonce": "0x0",
        "timestamp": "0x12345678",
        "extraData": "0x637573746f6d5f67656e657369735f74657374",
        "gasLimit": "0x1312D00",
        "baseFeePerGas": "0x1",
        "difficulty": "0x0",
        "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "coinbase": "0x0000000000000000000000000000000000000000",
        "alloc": {
            "0x8626f6940E2eb28930eFb4CeF49B2d1F2C9C1199": {
                "balance": "0xD3C21BCECCEDA1000000"
            },
            "0x5300000000000000000000000000000000000002": {
                "balance": "0xd3c21bcecceda1000000",
                "storage": {
                    "0x01": "0x000000000000000000000000000000000000000000000000000000003758e6b0",
                    "0x02": "0x0000000000000000000000000000000000000000000000000000000000000038",
                    "0x03": "0x000000000000000000000000000000000000000000000000000000003e95ba80",
                    "0x04": "0x0000000000000000000000005300000000000000000000000000000000000003",
                    "0x05": "0x000000000000000000000000000000000000000000000000000000008390c2c1",
                    "0x06": "0x00000000000000000000000000000000000000000000000000000069cf265bfe",
                    "0x07": "0x00000000000000000000000000000000000000000000000000000000168b9aa3"
                }
            }
        },
        "number": "0x0",
        "gasUsed": "0x0",
        "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
    }"#;

    let custom_genesis =
        serde_json::from_str(custom_genesis_json).expect("Custom genesis JSON should be valid");

    // Create a custom ScrollChainSpec using the from_custom_genesis method
    // This simulates what would happen when using --chain flag with a custom file
    let custom_chain_spec = Arc::new(ScrollChainSpec::from_custom_genesis(custom_genesis));

    // Launch 2 nodes: node0=sequencer and node1=follower.
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .with_chain_spec(custom_chain_spec.clone())
        .build()
        .await?;

    // Verify the genesis hash is different from all predefined networks
    assert_ne!(custom_chain_spec.genesis_hash(), SCROLL_DEV.genesis_hash());
    assert_ne!(custom_chain_spec.genesis_hash(), SCROLL_SEPOLIA.genesis_hash());
    assert_ne!(custom_chain_spec.genesis_hash(), SCROLL_MAINNET.genesis_hash());

    // Verify both nodes start with the same genesis hash from the custom chain spec
    assert_eq!(
        custom_chain_spec.genesis_hash(),
        fixture.get_sequencer_block().await?.header.hash_slow(),
        "Node0 should have the custom genesis hash"
    );
    assert_eq!(
        custom_chain_spec.genesis_hash(),
        fixture.get_block(1).await?.header.hash_slow(),
        "Node1 should have the custom genesis hash"
    );

    // Set L1 synced on sequencer node
    fixture.l1().for_node(0).sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Let the sequencer build 10 blocks.
    for _ in 1..=10 {
        let b = fixture.build_block().build_and_await_block().await?;
        tracing::info!(target: "scroll::test", block_number = ?b.header.number, block_hash = ?b.header.hash_slow(), "Sequenced block");
    }

    // Assert that the follower node has received all 10 blocks from the sequencer node.
    fixture
        .expect_event_on(1)
        .where_n_events(10, |e| matches!(e, ChainOrchestratorEvent::ChainExtended(_)))
        .await?;

    // Assert both nodes have the same latest block hash.
    assert_eq!(
        fixture.get_sequencer_block().await?.header.number,
        10,
        "Node0 should be at block 10"
    );
    assert_eq!(
        fixture.get_sequencer_block().await?.header.hash_slow(),
        fixture.get_block(1).await?.header.hash_slow(),
        "Both nodes should have the same latest block hash"
    );

    Ok(())
}

#[tokio::test]
async fn can_rpc_enable_disable_sequencing() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    color_eyre::install()?;

    // Launch sequencer node with automatic sequencing enabled.
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .block_time(40)
        .with_sequencer_auto_start(true)
        .build()
        .await?;

    // Set L1 synced
    fixture.l1().sync().await?;

    // Test that sequencing is initially enabled (blocks produced automatically)
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_ne!(fixture.get_sequencer_block().await?.header.number, 0, "Should produce blocks");

    // Disable automatic sequencing via RPC
    let client = fixture.sequencer().node.rpc_client().expect("Should have rpc client");
    let result = RollupNodeAdminApiClient::disable_automatic_sequencing(&client).await?;
    assert!(result, "Disable automatic sequencing should return true");

    // Wait a bit and verify no more blocks are produced automatically.
    // +1 blocks is okay due to still being processed
    let block_num_before_wait = fixture.get_sequencer_block().await?.header.number;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let block_num_after_wait = fixture.get_sequencer_block().await?.header.number;
    assert!(
        (block_num_before_wait..=block_num_before_wait + 1).contains(&block_num_after_wait),
        "No blocks should be produced automatically after disabling"
    );

    // Make sure follower is at same block
    fixture.expect_event_on(1).chain_extended(block_num_after_wait).await?;
    assert_eq!(block_num_after_wait, fixture.get_block(1).await?.header.number);

    // Verify manual block building still works
    fixture
        .build_block()
        .expect_block_number(block_num_after_wait + 1)
        .build_and_await_block()
        .await?;

    // Wait for the follower to import the block
    fixture.expect_event_on(1).chain_extended(block_num_after_wait + 1).await?;

    // Enable sequencing again
    let result = RollupNodeAdminApiClient::enable_automatic_sequencing(&client).await?;
    assert!(result, "Enable automatic sequencing should return true");

    // Make sure automatic sequencing resumes
    fixture.expect_event().block_sequenced(block_num_after_wait + 2).await?;
    fixture.expect_event_on(1).chain_extended(block_num_after_wait + 2).await?;

    Ok(())
}

/// Tests that a follower node correctly rejects L2 blocks containing L1 messages it hasn't received
/// yet.
///
/// This test verifies the security mechanism that prevents nodes from processing blocks with
/// unknown L1 messages, ensuring L2 chain consistency.
///
/// # Test scenario
/// 1. Sets up two nodes: a sequencer and a follower
/// 2. The sequencer builds 10 initial blocks that are successfully imported by the follower
/// 3. An L1 message is sent only to the sequencer (not to the follower)
/// 4. The sequencer includes this L1 message in block 11 and continues building blocks up to block
///    15
/// 5. The follower detects the unknown L1 message and stops processing at block 10
/// 6. Once the L1 message is finally sent to the follower, it can process the previously rejected
///    blocks
/// 7. The test confirms both nodes are synchronized at block 16 after the follower catches up
///
/// # Key verification points
/// - The follower correctly identifies missing L1 messages with a `L1MessageMissingInDatabase`
///   event
/// - Block processing halts at the last valid block when an unknown L1 message is encountered
/// - The follower can resume processing and catch up once it receives the missing L1 message
/// - This prevents nodes from accepting blocks with L1 messages they cannot validate
#[tokio::test]
async fn can_reject_l2_block_with_unknown_l1_message() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    color_eyre::install()?;

    // Launch 2 nodes: node0=sequencer and node1=follower.
    let mut fixture = TestFixture::builder().sequencer().followers(1).build().await?;

    // Set L1 synced
    fixture.l1().sync().await?;
    fixture.expect_event_on_all_nodes().l1_synced().await?;

    // Let the sequencer build 10 blocks before performing the reorg process.
    for i in 1..=10 {
        let b = fixture.build_block().expect_block_number(i).build_and_await_block().await?;
        tracing::info!(target: "scroll::test", block_number = ?b.header.number, block_hash = ?b.header.hash_slow(), "Sequenced block")
    }

    // Assert that the follower node has received all 10 blocks from the sequencer node.
    fixture.expect_event_on(1).chain_extended(10).await?;

    // Send the L1 message to the sequencer node but not to follower node.
    fixture
        .l1()
        .for_node(0)
        .add_message()
        .at_block(10)
        .sender(address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"))
        .value(0)
        .to(Address::ZERO)
        .send()
        .await?;
    fixture.expect_event().l1_message_committed().await?;

    fixture.l1().for_node(0).new_block(10).await?;
    fixture.expect_event().new_l1_block().await?;

    // Build block that contains the L1 message.
    fixture
        .build_block()
        .expect_block_number(11)
        .expect_l1_message()
        .build_and_await_block()
        .await?;

    for i in 12..=15 {
        fixture.build_block().expect_block_number(i).build_and_await_block().await?;
    }

    fixture
        .expect_event_on(1)
        .where_event(|e| {
            matches!(
                e,
                ChainOrchestratorEvent::L1MessageNotFoundInDatabase(L1MessageKey::TransactionHash(
                    hash
                )) if hash == &b256!("0x0a2f8e75392ab51a26a2af835042c614eb141cd934fe1bdd4934c10f2fe17e98")
            )
        })
        .await?;

    // follower node should not import block 15
    // follower node doesn't know about the L1 message so stops processing the chain at block 10
    assert_eq!(fixture.get_block(1).await?.header.number, 10);

    // Finally send L1 the L1 message to follower node.
    fixture
        .l1()
        .for_node(1)
        .add_message()
        .at_block(10)
        .sender(address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"))
        .value(0)
        .to(Address::ZERO)
        .send()
        .await?;
    fixture.expect_event_on(1).l1_message_committed().await?;

    fixture.l1().for_node(1).new_block(10).await?;
    fixture.expect_event_on(1).new_l1_block().await?;

    // Produce another block and send to follower node.
    fixture.build_block().expect_block_number(16).build_and_await_block().await?;

    // Assert that the follower node has received the latest block from the sequencer node and
    // processed the missing chain before.
    // This is possible now because it has received the L1 message.
    fixture.expect_event_on(1).chain_extended(16).await?;

    // Assert both nodes are at block 16.
    let node0_latest_block = fixture.get_sequencer_block().await?;
    assert_eq!(node0_latest_block.header.number, 16);
    assert_eq!(
        node0_latest_block.header.hash_slow(),
        fixture.get_block(1).await?.header.hash_slow()
    );

    Ok(())
}

#[tokio::test]
async fn can_gossip_over_eth_wire() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(1)
        .with_sequencer_auto_start(true)
        .block_time(40)
        .with_scroll_wire(false)
        .build()
        .await?;

    // Set the L1 synced on the sequencer node to start block production.
    fixture.l1().for_node(0).sync().await?;
    fixture.expect_event().l1_synced().await?;

    let mut eth_wire_blocks =
        fixture.follower(0).node.inner.network.eth_wire_block_listener().await?;

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

    let mut fixture1 = TestFixture::builder()
        .sequencer()
        .followers(1)
        .with_test(false)
        .with_consensus_system_contract(signer_1_address)
        .with_signer(signer_1)
        .with_sequencer_auto_start(true)
        .with_eth_scroll_bridge(false)
        .block_time(40)
        .build()
        .await?;

    let mut fixture2 = TestFixture::builder()
        .sequencer()
        .with_test(false)
        .with_consensus_system_contract(signer_1_address)
        .with_signer(signer_2)
        .with_sequencer_auto_start(true)
        .with_eth_scroll_bridge(false)
        .block_time(40)
        .build()
        .await?;

    // Set the L1 synced on both nodes to start block production.
    fixture1.l1().for_node(0).sync().await?;
    fixture2.l1().for_node(0).sync().await?;

    // connect the two sequencers
    fixture1.sequencer().node.connect(&mut fixture2.sequencer().node).await;

    // wait for 5 blocks to be produced.
    for i in 1..=5 {
        fixture1
            .expect_event_on(1)
            .where_event(|event| {
                if let ChainOrchestratorEvent::NewBlockReceived(block) = event {
                    let signature = block.signature;
                    let hash = sig_encode_hash(&block.block);
                    // Verify that the block is signed by the first sequencer.
                    let recovered_address = signature.recover_address_from_prehash(&hash).unwrap();
                    recovered_address == signer_1_address
                } else {
                    false
                }
            })
            .await?;
        fixture1.expect_event_on(1).chain_extended(i).await?;
    }

    fixture2.expect_event().chain_extended(5).await?;

    // now update the authorized signer to sequencer 2
    fixture1.l1().signer_update(signer_2_address).await?;
    fixture2.l1().signer_update(signer_2_address).await?;

    tokio::time::sleep(Duration::from_secs(1)).await;

    fixture1
        .expect_event_on(1)
        .where_n_events(5, |event| {
            if let ChainOrchestratorEvent::NewBlockReceived(block) = event {
                let signature = block.signature;
                let hash = sig_encode_hash(&block.block);
                let recovered_address = signature.recover_address_from_prehash(&hash).unwrap();
                // Verify that the block is signed by the second sequencer.
                recovered_address == signer_2_address
            } else {
                false
            }
        })
        .await?;

    Ok(())
}

/// Read the file provided at `path` as a [`Bytes`].
pub fn read_to_bytes<P: AsRef<std::path::Path>>(path: P) -> eyre::Result<Bytes> {
    use std::str::FromStr;
    Ok(Bytes::from_str(&std::fs::read_to_string(path)?)?)
}

/// Waits for n events to be emitted.
async fn wait_n_events(
    events: &mut EventStream<ChainOrchestratorEvent>,
    mut matches: impl FnMut(ChainOrchestratorEvent) -> bool,
    mut n: u64,
) {
    // TODO: refactor using `wait_for_event_predicate`
    while let Some(event) = events.next().await {
        if matches(event) {
            n -= 1;
        }
        if n == 0 {
            break
        }
    }
}

/// Helper function to wait until a predicate is true or a timeout occurs.
pub async fn eventually<F, Fut>(timeout: Duration, tick: Duration, message: &str, mut predicate: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
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
