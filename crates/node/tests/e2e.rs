//! End-to-end tests for the rollup node.

use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{address, b256, Address, Bytes, Signature, B256, U256};
use alloy_rpc_types_eth::Block;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use eyre::Ok;
use futures::StreamExt;
use reth_chainspec::EthChainSpec;
use reth_e2e_test_utils::{NodeHelperType, TmpDB};
use reth_network::{NetworkConfigBuilder, NetworkEventListenerProvider, Peers, PeersInfo};
use reth_network_api::block::EthWireProvider;
use reth_node_api::NodeTypesWithDBAdapter;
use reth_provider::providers::BlockchainProvider;
use reth_rpc_api::EthApiServer;
use reth_scroll_chainspec::{ScrollChainSpec, SCROLL_DEV, SCROLL_MAINNET, SCROLL_SEPOLIA};
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_scroll_primitives::ScrollBlock;
use reth_tokio_util::EventStream;
use rollup_node::{
    constants::SCROLL_GAS_LIMIT,
    test_utils::{
        default_sequencer_test_scroll_rollup_node_config, default_test_scroll_rollup_node_config,
        generate_tx, setup_engine,
    },
    BeaconProviderArgs, ChainOrchestratorArgs, ConsensusAlgorithm, ConsensusArgs, DatabaseArgs,
    EngineDriverArgs, GasPriceOracleArgs, L1ProviderArgs, NetworkArgs as ScrollNetworkArgs,
    RollupNodeContext, RollupNodeExtApiClient, ScrollRollupNode, ScrollRollupNodeConfig,
    SequencerArgs,
};
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use rollup_node_manager::{RollupManagerCommand, RollupManagerEvent};
use rollup_node_primitives::{sig_encode_hash, BatchCommitData, ConsensusUpdate};
use rollup_node_providers::BlobSource;
use rollup_node_sequencer::L1MessageInclusionMode;
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_rpc_types::Transaction as ScrollAlloyTransaction;
use scroll_db::L1MessageStart;
use scroll_network::NewBlockWithPeer;
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
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            auto_start: true,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            allow_empty_blocks: true,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
        gas_price_oracle_args: GasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
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

    wait_n_events(
        &mut rnm_events,
        |e| {
            if let RollupManagerEvent::ChainOrchestratorEvent(
                rollup_node_chain_orchestrator::ChainOrchestratorEvent::L1MessageCommitted(index),
            ) = e
            {
                assert_eq!(index, 0);
                true
            } else {
                false
            }
        },
        1,
    )
    .await;

    rnm_handle.build_block().await;

    wait_n_events(
        &mut rnm_events,
        |e| {
            if let RollupManagerEvent::BlockSequenced(block) = e {
                assert_eq!(block.body.transactions.len(), 1);
                assert_eq!(
                    block.body.transactions[0].as_l1_message().unwrap().inner(),
                    &l1_message
                );
                true
            } else {
                false
            }
        },
        1,
    )
    .await;

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
            signer_address: None,
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            auto_start: true,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            allow_empty_blocks: true,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
        gas_price_oracle_args: GasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
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
    wait_n_events(
        &mut sequencer_events,
        |e| {
            if let RollupManagerEvent::BlockSequenced(block) = e {
                assert_eq!(block.body.transactions.len(), 1);
                true
            } else {
                false
            }
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

    // assert that a chain extension is triggered on the follower node
    wait_n_events(
        &mut follower_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(
                    rollup_node_chain_orchestrator::ChainOrchestratorEvent::ChainExtended(_)
                )
            )
        },
        1,
    )
    .await;

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
            signer_address: None,
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            auto_start: true,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            allow_empty_blocks: true,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
        gas_price_oracle_args: GasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        database: None,
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
    let mut block = ScrollBlock::default();
    block.header.number = 1;
    block.header.parent_hash =
        b256!("0x14844a4fc967096c628e90df3bb0c3e98941bdd31d1982c2f3e70ed17250d98b");

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

    let mut test_config = default_sequencer_test_scroll_rollup_node_config();
    test_config.consensus_args.algorithm = ConsensusAlgorithm::SystemContract;
    test_config.consensus_args.authorized_signer = Some(authorized_address);
    test_config.signer_args.private_key = Some(authorized_signer.clone());

    // Setup nodes
    let (mut nodes, _tasks, _) =
        setup_engine(test_config, 2, chain_spec.clone(), false, false).await.unwrap();

    let node0 = nodes.remove(0);
    let node1 = nodes.remove(0);

    // Get handles
    let node0_rmn_handle = node0.inner.add_ons_handle.rollup_manager_handle.clone();
    let node0_network_handle = node0_rmn_handle.get_network_handle().await.unwrap();
    let node0_id = node0_network_handle.inner().peer_id();

    let node1_rnm_handle = node1.inner.add_ons_handle.rollup_manager_handle.clone();
    let node1_network_handle = node1_rnm_handle.get_network_handle().await.unwrap();

    // Get event streams
    let mut node0_events = node0_rmn_handle.get_event_listener().await.unwrap();
    let mut node1_events = node1_rnm_handle.get_event_listener().await.unwrap();

    // === Phase 1: Test valid block with correct signature ===

    // Have the legitimate sequencer build and sign a block
    node0_rmn_handle.build_block().await;

    // Wait for the sequencer to build the block
    let block0 = if let Some(RollupManagerEvent::BlockSequenced(block)) = node0_events.next().await
    {
        assert_eq!(block.body.transactions.len(), 0, "Block should have no transactions");
        block
    } else {
        panic!("Failed to receive block from sequencer");
    };

    // Node1 should receive and accept the valid block
    if let Some(RollupManagerEvent::NewBlockReceived(block_with_peer)) = node1_events.next().await {
        assert_eq!(block0.hash_slow(), block_with_peer.block.hash_slow());

        // Verify the signature is from the authorized signer
        let hash = sig_encode_hash(&block_with_peer.block);
        let recovered = block_with_peer.signature.recover_address_from_prehash(&hash).unwrap();
        assert_eq!(recovered, authorized_address, "Block should be signed by authorized signer");
    } else {
        panic!("Failed to receive valid block at follower");
    }

    // Wait for successful import
    wait_n_events(&mut node1_events, |e| matches!(e, RollupManagerEvent::BlockImported(_)), 1)
        .await;

    // === Phase 2: Create and send valid block with unauthorized signer signature ===

    // Get initial reputation of node0 from node1's perspective
    let initial_reputation =
        node1_network_handle.inner().reputation_by_id(*node0_id).await.unwrap().unwrap();
    assert_eq!(initial_reputation, 0, "Initial reputation should be zero");

    // Create a new block manually (we'll reuse the valid block structure but with wrong signature)
    let mut block1 = block0.clone();
    block1.header.number += 1;
    block1.header.parent_hash = block0.hash_slow();
    block1.header.timestamp += 1;

    // Sign the block with the unauthorized signer
    let block_hash = sig_encode_hash(&block1);
    let unauthorized_signature = unauthorized_signer.sign_hash(&block_hash).await.unwrap();

    // Send the block with invalid signature from node0 to node1
    node0_network_handle.announce_block(block1.clone(), unauthorized_signature);

    // Node1 should receive and process the invalid block
    wait_for_event_predicate_5s(&mut node1_events, |e| {
        if let RollupManagerEvent::NewBlockReceived(block_with_peer) = e {
            assert_eq!(block1.hash_slow(), block_with_peer.block.hash_slow());

            // Verify the signature is from the unauthorized signer
            let hash = sig_encode_hash(&block_with_peer.block);
            let recovered = block_with_peer.signature.recover_address_from_prehash(&hash).unwrap();
            return recovered == unauthorized_signer.address();
        }
        false
    })
    .await?;

    eventually(
        Duration::from_secs(5),
        Duration::from_millis(100),
        "Node0 reputation should be lower after sending block with invalid signature",
        || async {
            let current_reputation =
                node1_network_handle.inner().reputation_by_id(*node0_id).await.unwrap().unwrap();
            current_reputation < initial_reputation
        },
    )
    .await;

    // === Phase 3: Send valid block with invalid signature ===
    // Get current reputation of node0 from node1's perspective
    let current_reputation =
        node1_network_handle.inner().reputation_by_id(*node0_id).await.unwrap().unwrap();

    let invalid_signature = Signature::new(U256::from(1), U256::from(1), false);

    // Create a new block with the same structure as before but with an invalid signature.
    // We need to make sure the block is different so that it is not filtered.
    block1.header.timestamp += 1;
    node0_network_handle.announce_block(block1.clone(), invalid_signature);

    eventually(
        Duration::from_secs(5),
        Duration::from_millis(100),
        "Node0 reputation should be lower after sending block with invalid signature",
        || async {
            let all_peers = node1_network_handle.inner().get_all_peers().await.unwrap();
            if all_peers.is_empty() {
                return true; // No peers to check, assume penalization and peer0 is blocked and
                             // disconnected
            }

            let penalized_reputation =
                node1_network_handle.inner().reputation_by_id(*node0_id).await.unwrap().unwrap();
            penalized_reputation < current_reputation
        },
    )
    .await;

    Ok(())
}

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn can_forward_tx_to_sequencer() {
    reth_tracing::init_test_tracing();

    // create 2 nodes
    let mut sequencer_node_config = default_sequencer_test_scroll_rollup_node_config();
    sequencer_node_config.sequencer_args.block_time = 0;
    sequencer_node_config.network_args.enable_eth_scroll_wire_bridge = false;
    let mut follower_node_config = default_test_scroll_rollup_node_config();
    follower_node_config.network_args.enable_eth_scroll_wire_bridge = false;

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

    // skip the chain committed event
    let _ = follower_events.next().await;

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

    // assert that a chain extension is triggered on the follower node
    wait_n_events(
        &mut follower_events,
        |e| {
            matches!(
                e,
                RollupManagerEvent::ChainOrchestratorEvent(
                    rollup_node_chain_orchestrator::ChainOrchestratorEvent::ChainExtended(_)
                )
            )
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

    // skip the chain committed event
    let _ = follower_events.next().await;

    // assert that the follower node has received the block from the peer
    if let Some(RollupManagerEvent::NewBlockReceived(block_with_peer)) =
        follower_events.next().await
    {
        assert_eq!(block_with_peer.block.body.transactions.len(), 1);
    } else {
        panic!("Failed to receive block from rollup node");
    }

    // skip the chain extension event
    let _ = follower_events.next().await;

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
        chain_spec.clone(),
        network_config,
        scroll_wire_config,
        None,
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
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec.clone(), false, false)
            .await?;
    let node = nodes.pop().unwrap();

    // Instantiate the rollup node manager.
    let mut config = default_test_scroll_rollup_node_config();
    let path = node.inner.config.datadir().db().join("scroll.db?mode=rwc");
    let path = PathBuf::from("sqlite://".to_string() + &*path.to_string_lossy());
    config.beacon_provider_args.url = Some(
        "http://dummy:8545"
            .parse()
            .expect("valid url that will not be used as test batches use calldata"),
    );
    config.hydrate(node.inner.config.clone()).await?;

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

    // Send the first batch commit to the rollup node manager and finalize it.
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_0_data.clone()))).await?;
    l1_notification_tx
        .send(Arc::new(L1Notification::BatchFinalization {
            hash: batch_0_data.hash,
            index: batch_0_data.index,
            block_number: batch_0_data.block_number,
        }))
        .await?;

    // Lets finalize the first batch
    l1_notification_tx.send(Arc::new(L1Notification::Finalized(batch_0_data.block_number))).await?;

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

    // Now we send the second batch commit and finalize it.
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_1_data.clone()))).await?;
    l1_notification_tx
        .send(Arc::new(L1Notification::BatchFinalization {
            hash: batch_1_data.hash,
            index: batch_1_data.index,
            block_number: batch_1_data.block_number,
        }))
        .await?;

    // Lets finalize the second batch.
    l1_notification_tx.send(Arc::new(L1Notification::Finalized(batch_1_data.block_number))).await?;

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
    l1_notification_tx.send(Arc::new(L1Notification::BatchCommit(batch_1_data.clone()))).await?;
    l1_notification_tx
        .send(Arc::new(L1Notification::BatchFinalization {
            hash: batch_1_data.hash,
            index: batch_1_data.index,
            block_number: batch_1_data.block_number,
        }))
        .await?;
    // Lets finalize the second batch.
    l1_notification_tx.send(Arc::new(L1Notification::Finalized(batch_1_data.block_number))).await?;

    // Lets fetch the first consolidated block event - this should be the first block of the batch.
    let l2_block = loop {
        if let Some(RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome)) =
            rnm_events.next().await
        {
            break consolidation_outcome.block_info().clone();
        }
    };

    // One issue #273 is completed, we will again have safe blocks != finalized blocks, and this
    // should be changed to 1. Assert that the consolidated block is the first block of the
    // batch.
    assert_eq!(
        l2_block.block_info.number, 5,
        "Consolidated block number does not match expected number"
    );

    // Lets now iterate over all remaining blocks expected to be derived from the second batch
    // commit.
    for i in 6..=57 {
        loop {
            if let Some(RollupManagerEvent::L1DerivedBlockConsolidated(consolidation_outcome)) =
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

#[tokio::test]
#[ignore = "Enable once we implement issue #273"]
async fn can_handle_batch_revert() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    let chain_spec = (*SCROLL_MAINNET).clone();

    // Launch a node
    let (mut nodes, _tasks, _) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec.clone(), false, false)
            .await?;
    let node = nodes.pop().unwrap();
    let handle = node.inner.add_ons_handle.rollup_manager_handle.clone();
    let l1_watcher_tx = node.inner.add_ons_handle.l1_watcher_tx.clone().unwrap();

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
    tokio::time::sleep(Duration::from_millis(300)).await;

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
async fn can_handle_l1_message_reorg() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    color_eyre::install()?;
    let chain_spec = (*SCROLL_DEV).clone();

    // Launch 2 nodes: node0=sequencer and node1=follower.
    let config = default_sequencer_test_scroll_rollup_node_config();
    let (mut nodes, _tasks, _) = setup_engine(config, 2, chain_spec.clone(), false, false).await?;
    let node0 = nodes.remove(0);
    let node1 = nodes.remove(0);

    // Get handles
    let node0_rnm_handle = node0.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut node0_rnm_events = node0_rnm_handle.get_event_listener().await?;
    let node0_l1_watcher_tx = node0.inner.add_ons_handle.l1_watcher_tx.as_ref().unwrap();

    let node1_rnm_handle = node1.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut node1_rnm_events = node1_rnm_handle.get_event_listener().await?;
    let node1_l1_watcher_tx = node1.inner.add_ons_handle.l1_watcher_tx.as_ref().unwrap();

    // Let the sequencer build 10 blocks before performing the reorg process.
    for i in 1..=10 {
        node0_rnm_handle.build_block().await;
        let b = wait_for_block_sequenced_5s(&mut node0_rnm_events, i).await?;
        tracing::info!(target: "scroll::test", block_number = ?b.header.number, block_hash = ?b.header.hash_slow(), "Sequenced block");
    }

    // Assert that the follower node has received all 10 blocks from the sequencer node.
    wait_for_block_imported_5s(&mut node1_rnm_events, 10).await?;

    // Send a L1 message and wait for it to be indexed.
    let l1_message_notification = L1Notification::L1Message {
        message: TxL1Message {
            queue_index: 0,
            gas_limit: 21000,
            to: Default::default(),
            value: Default::default(),
            sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            input: Default::default(),
        },
        block_number: 10,
        block_timestamp: 0,
    };

    // Send the L1 message to the sequencer node.
    node0_l1_watcher_tx.send(Arc::new(l1_message_notification.clone())).await?;
    node0_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(10))).await?;
    wait_for_event_5s(
        &mut node0_rnm_events,
        RollupManagerEvent::ChainOrchestratorEvent(ChainOrchestratorEvent::L1MessageCommitted(0)),
    )
    .await?;

    // Send L1 the L1 message to follower node.
    node1_l1_watcher_tx.send(Arc::new(l1_message_notification)).await?;
    node1_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(10))).await?;
    wait_for_event_5s(
        &mut node1_rnm_events,
        RollupManagerEvent::ChainOrchestratorEvent(ChainOrchestratorEvent::L1MessageCommitted(0)),
    )
    .await?;

    // Build block that contains the L1 message.
    let mut block11_before_reorg = None;
    node0_rnm_handle.build_block().await;
    wait_for_event_predicate_5s(&mut node0_rnm_events, |e| {
        if let RollupManagerEvent::BlockSequenced(block) = e {
            if block.header.number == 11 &&
                block.body.transactions.len() == 1 &&
                block.body.transactions.iter().any(|tx| tx.is_l1_message())
            {
                block11_before_reorg = Some(block.header.hash_slow());
                return true;
            }
        }

        false
    })
    .await?;

    for i in 12..=15 {
        node0_rnm_handle.build_block().await;
        wait_for_block_sequenced_5s(&mut node0_rnm_events, i).await?;
    }

    // Assert that the follower node has received the latest block from the sequencer node.
    wait_for_block_imported_5s(&mut node1_rnm_events, 15).await?;

    // Assert both nodes are at block 15.
    let node0_latest_block = latest_block(&node0).await?;
    assert_eq!(node0_latest_block.header.number, 15);
    assert_eq!(
        node0_latest_block.header.hash_slow(),
        latest_block(&node1).await?.header.hash_slow()
    );

    // Issue and wait for the reorg.
    node0_l1_watcher_tx.send(Arc::new(L1Notification::Reorg(9))).await?;
    wait_for_event_5s(&mut node0_rnm_events, RollupManagerEvent::Reorg(9)).await?;
    node1_l1_watcher_tx.send(Arc::new(L1Notification::Reorg(9))).await?;
    wait_for_event_5s(&mut node1_rnm_events, RollupManagerEvent::Reorg(9)).await?;

    // Since the L1 reorg reverted the L1 message included in block 11, the sequencer
    // should produce a new block at height 11.
    node0_rnm_handle.build_block().await;
    wait_for_block_sequenced_5s(&mut node0_rnm_events, 11).await?;

    // Assert that the follower node has received the new block from the sequencer node.
    wait_for_block_imported_5s(&mut node1_rnm_events, 11).await?;

    // Assert ChainOrchestrator finished processing block.
    wait_for_chain_committed_5s(&mut node0_rnm_events, 11, true).await?;
    wait_for_chain_committed_5s(&mut node1_rnm_events, 11, true).await?;

    // Assert both nodes are at block 11.
    assert_latest_block_on_rpc_by_number(&node0, 11).await;
    let node0_latest_block = latest_block(&node0).await?;
    assert_latest_block_on_rpc_by_hash(&node1, node0_latest_block.header.hash_slow()).await;

    // Assert that block 11 has a different hash after the reorg.
    assert_ne!(block11_before_reorg.unwrap(), node0_latest_block.header.hash_slow());

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
    let config = default_sequencer_test_scroll_rollup_node_config();
    let (mut nodes, _tasks, _) =
        setup_engine(config, 2, custom_chain_spec.clone(), false, false).await?;
    let node0 = nodes.remove(0);
    let node1 = nodes.remove(0);

    // Get handles
    let node0_rnm_handle = node0.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut node0_rnm_events = node0_rnm_handle.get_event_listener().await?;

    let node1_rnm_handle = node1.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut node1_rnm_events = node1_rnm_handle.get_event_listener().await?;

    // Verify the genesis hash is different from all predefined networks
    assert_ne!(custom_chain_spec.genesis_hash(), SCROLL_DEV.genesis_hash());
    assert_ne!(custom_chain_spec.genesis_hash(), SCROLL_SEPOLIA.genesis_hash());
    assert_ne!(custom_chain_spec.genesis_hash(), SCROLL_MAINNET.genesis_hash());

    // Verify both nodes start with the same genesis hash from the custom chain spec
    assert_eq!(
        custom_chain_spec.genesis_hash(),
        node0.block_hash(0),
        "Node0 should have the custom genesis hash"
    );
    assert_eq!(
        custom_chain_spec.genesis_hash(),
        node1.block_hash(0),
        "Node1 should have the custom genesis hash"
    );

    // Let the sequencer build 10 blocks.
    for i in 1..=10 {
        node0_rnm_handle.build_block().await;
        let b = wait_for_block_sequenced_5s(&mut node0_rnm_events, i).await?;
        tracing::info!(target: "scroll::test", block_number = ?b.header.number, block_hash = ?b.header.hash_slow(), "Sequenced block");
    }

    // Assert that the follower node has received all 10 blocks from the sequencer node.
    wait_for_block_imported_5s(&mut node1_rnm_events, 10).await?;

    // Assert both nodes have the same latest block hash.
    assert_eq!(latest_block(&node0).await?.header.number, 10, "Node0 should be at block 10");
    assert_eq!(
        latest_block(&node0).await?.header.hash_slow(),
        latest_block(&node1).await?.header.hash_slow(),
        "Both nodes should have the same latest block hash"
    );

    Ok(())
}

#[tokio::test]
async fn can_rpc_enable_disable_sequencing() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();
    color_eyre::install()?;
    let chain_spec = (*SCROLL_DEV).clone();

    // Launch sequencer node with automatic sequencing enabled.
    let mut config = default_sequencer_test_scroll_rollup_node_config();
    config.sequencer_args.block_time = 40; // Enable automatic block production

    let (mut nodes, _tasks, _) = setup_engine(config, 2, chain_spec.clone(), false, false).await?;
    let node0 = nodes.remove(0);
    let node1 = nodes.remove(0);

    // Get handles
    let node0_rnm_handle = node0.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut node0_rnm_events = node0_rnm_handle.get_event_listener().await?;

    let node1_rnm_handle = node1.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut node1_rnm_events = node1_rnm_handle.get_event_listener().await?;

    // Create RPC client
    let client0 = node0.rpc_client().expect("RPC client should be available");

    // Test that sequencing is initially enabled (blocks produced automatically)
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_ne!(latest_block(&node0).await?.header.number, 0, "Should produce blocks");

    // Disable automatic sequencing via RPC
    let result = RollupNodeExtApiClient::disable_automatic_sequencing(&client0).await?;
    assert!(result, "Disable automatic sequencing should return true");

    // Wait a bit and verify no more blocks are produced automatically.
    // +1 blocks is okay due to still being processed
    let block_num_before_wait = latest_block(&node0).await?.header.number;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let block_num_after_wait = latest_block(&node0).await?.header.number;
    assert!(
        (block_num_before_wait..=block_num_before_wait + 1).contains(&block_num_after_wait),
        "No blocks should be produced automatically after disabling"
    );

    // Make sure follower is at same block
    wait_for_block_imported_5s(&mut node1_rnm_events, block_num_after_wait).await?;
    assert_eq!(block_num_after_wait, latest_block(&node1).await?.header.number);

    // Verify manual block building still works
    node0_rnm_handle.build_block().await;
    wait_for_block_sequenced_5s(&mut node0_rnm_events, block_num_after_wait + 1).await?;

    // Wait for the follower to import the block
    wait_for_block_imported_5s(&mut node1_rnm_events, block_num_after_wait + 1).await?;

    // Enable sequencing again
    let result = RollupNodeExtApiClient::enable_automatic_sequencing(&client0).await?;
    assert!(result, "Enable automatic sequencing should return true");

    // Make sure automatic sequencing resumes
    wait_for_block_sequenced_5s(&mut node0_rnm_events, block_num_after_wait + 2).await?;
    wait_for_block_imported_5s(&mut node1_rnm_events, block_num_after_wait + 2).await?;

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
    let chain_spec = (*SCROLL_DEV).clone();

    // Launch 2 nodes: node0=sequencer and node1=follower.
    let config = default_sequencer_test_scroll_rollup_node_config();
    let (mut nodes, _tasks, _) = setup_engine(config, 2, chain_spec.clone(), false, false).await?;
    let node0 = nodes.remove(0);
    let node1 = nodes.remove(0);

    // Get handles
    let node0_rnm_handle = node0.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut node0_rnm_events = node0_rnm_handle.get_event_listener().await?;
    let node0_l1_watcher_tx = node0.inner.add_ons_handle.l1_watcher_tx.as_ref().unwrap();

    let node1_rnm_handle = node1.inner.add_ons_handle.rollup_manager_handle.clone();
    let mut node1_rnm_events = node1_rnm_handle.get_event_listener().await?;
    let node1_l1_watcher_tx = node1.inner.add_ons_handle.l1_watcher_tx.as_ref().unwrap();

    // Let the sequencer build 10 blocks before performing the reorg process.
    for i in 1..=10 {
        node0_rnm_handle.build_block().await;
        let b = wait_for_block_sequenced_5s(&mut node0_rnm_events, i).await?;
        tracing::info!(target: "scroll::test", block_number = ?b.header.number, block_hash = ?b.header.hash_slow(), "Sequenced block")
    }

    // Assert that the follower node has received all 10 blocks from the sequencer node.
    wait_for_block_imported_5s(&mut node1_rnm_events, 10).await?;

    // Send a L1 message and wait for it to be indexed.
    let l1_message_notification = L1Notification::L1Message {
        message: TxL1Message {
            queue_index: 0,
            gas_limit: 21000,
            to: Default::default(),
            value: Default::default(),
            sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            input: Default::default(),
        },
        block_number: 10,
        block_timestamp: 0,
    };

    // Send the L1 message to the sequencer node but not to follower node.
    node0_l1_watcher_tx.send(Arc::new(l1_message_notification.clone())).await?;
    node0_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(10))).await?;
    wait_for_event_5s(
        &mut node0_rnm_events,
        RollupManagerEvent::ChainOrchestratorEvent(ChainOrchestratorEvent::L1MessageCommitted(0)),
    )
    .await?;

    // Build block that contains the L1 message.
    node0_rnm_handle.build_block().await;
    wait_for_event_predicate_5s(&mut node0_rnm_events, |e| {
        if let RollupManagerEvent::BlockSequenced(block) = e {
            if block.header.number == 11 &&
                block.body.transactions.len() == 1 &&
                block.body.transactions.iter().any(|tx| tx.is_l1_message())
            {
                return true;
            }
        }

        false
    })
    .await?;

    for i in 12..=15 {
        node0_rnm_handle.build_block().await;
        wait_for_block_sequenced_5s(&mut node0_rnm_events, i).await?;
    }

    wait_for_event_5s(
        &mut node1_rnm_events,
        RollupManagerEvent::L1MessageMissingInDatabase {
            start: L1MessageStart::Hash(b256!(
                "0x0a2f8e75392ab51a26a2af835042c614eb141cd934fe1bdd4934c10f2fe17e98"
            )),
        },
    )
    .await?;

    // follower node should not import block 15
    // follower node doesn't know about the L1 message so stops processing the chain at block 10
    assert_eq!(latest_block(&node1).await?.header.number, 10);

    // Finally send L1 the L1 message to follower node.
    node1_l1_watcher_tx.send(Arc::new(l1_message_notification)).await?;
    node1_l1_watcher_tx.send(Arc::new(L1Notification::NewBlock(10))).await?;
    wait_for_event_5s(
        &mut node1_rnm_events,
        RollupManagerEvent::ChainOrchestratorEvent(ChainOrchestratorEvent::L1MessageCommitted(0)),
    )
    .await?;

    // Produce another block and send to follower node.
    node0_rnm_handle.build_block().await;
    wait_for_block_sequenced_5s(&mut node0_rnm_events, 16).await?;

    // Assert that the follower node has received the latest block from the sequencer node and
    // processed the missing chain before.
    // This is possible now because it has received the L1 message.
    wait_for_block_imported_5s(&mut node1_rnm_events, 16).await?;

    // Assert both nodes are at block 16.
    let node0_latest_block = latest_block(&node0).await?;
    assert_eq!(node0_latest_block.header.number, 16);
    assert_eq!(
        node0_latest_block.header.hash_slow(),
        latest_block(&node1).await?.header.hash_slow()
    );

    Ok(())
}

#[tokio::test]
async fn can_gossip_over_eth_wire() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll dev with Feynman activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();

    let mut config = default_sequencer_test_scroll_rollup_node_config();
    config.sequencer_args.block_time = 40;

    // Setup the rollup node manager.
    let (mut nodes, _tasks, _) =
        setup_engine(config, 2, chain_spec.clone(), false, false).await.unwrap();
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
    sequencer_1_config.sequencer_args.block_time = 40;
    sequencer_1_config.network_args.enable_eth_scroll_wire_bridge = false;

    let mut sequencer_2_config = default_sequencer_test_scroll_rollup_node_config();
    sequencer_2_config.test = false;
    sequencer_2_config.consensus_args.algorithm = ConsensusAlgorithm::SystemContract;
    sequencer_2_config.consensus_args.authorized_signer = Some(signer_1_address);
    sequencer_2_config.signer_args.private_key = Some(signer_2);
    sequencer_2_config.sequencer_args.block_time = 40;
    sequencer_2_config.network_args.enable_eth_scroll_wire_bridge = false;

    // Setup two sequencer nodes.
    let (mut nodes, _tasks, _) =
        setup_engine(sequencer_1_config, 2, chain_spec.clone(), false, false).await.unwrap();
    let follower = nodes.pop().unwrap();
    let mut sequencer_1 = nodes.pop().unwrap();
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
    let mut sequencer_2_events =
        sequencer_2.inner.add_ons_handle.rollup_manager_handle.get_event_listener().await.unwrap();

    // connect the two sequencers
    sequencer_1.connect(&mut sequencer_2).await;

    for _ in 0..5 {
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
            1,
        )
        .await;
        wait_n_events(
            &mut follower_events,
            |event| matches!(event, RollupManagerEvent::BlockImported(_)),
            1,
        )
        .await;
    }

    wait_n_events(
        &mut sequencer_2_events,
        |e| matches!(e, RollupManagerEvent::BlockImported(_)),
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

async fn latest_block(
    node: &NodeHelperType<
        ScrollRollupNode,
        BlockchainProvider<NodeTypesWithDBAdapter<ScrollRollupNode, TmpDB>>,
    >,
) -> eyre::Result<Block<ScrollAlloyTransaction>> {
    node.rpc
        .inner
        .eth_api()
        .block_by_number(BlockNumberOrTag::Latest, false)
        .await?
        .ok_or_else(|| eyre::eyre!("Latest block not found"))
}

async fn wait_for_block_sequenced(
    events: &mut EventStream<RollupManagerEvent>,
    block_number: u64,
    timeout: Duration,
) -> eyre::Result<ScrollBlock> {
    let mut block = None;

    wait_for_event_predicate(
        events,
        |e| {
            if let RollupManagerEvent::BlockSequenced(b) = e {
                if b.header.number == block_number {
                    block = Some(b);
                    return true;
                }
            }

            false
        },
        timeout,
    )
    .await?;

    block.ok_or_else(|| eyre::eyre!("Block with number {block_number} was not sequenced"))
}

async fn wait_for_block_sequenced_5s(
    events: &mut EventStream<RollupManagerEvent>,
    block_number: u64,
) -> eyre::Result<ScrollBlock> {
    wait_for_block_sequenced(events, block_number, Duration::from_secs(5)).await
}

async fn wait_for_block_imported(
    events: &mut EventStream<RollupManagerEvent>,
    block_number: u64,
    timeout: Duration,
) -> eyre::Result<ScrollBlock> {
    let mut block = None;

    wait_for_event_predicate(
        events,
        |e| {
            if let RollupManagerEvent::BlockImported(b) = e {
                if b.header.number == block_number {
                    block = Some(b);
                    return true;
                }
            }

            false
        },
        timeout,
    )
    .await?;

    block.ok_or_else(|| eyre::eyre!("Block with number {block_number} was not imported"))
}

async fn wait_for_block_imported_5s(
    events: &mut EventStream<RollupManagerEvent>,
    block_number: u64,
) -> eyre::Result<ScrollBlock> {
    wait_for_block_imported(events, block_number, Duration::from_secs(5)).await
}

async fn wait_for_chain_committed_5s(
    events: &mut EventStream<RollupManagerEvent>,
    expected_block_number: u64,
    expected_consolidated: bool,
) -> eyre::Result<()> {
    wait_for_chain_committed(
        events,
        expected_block_number,
        expected_consolidated,
        Duration::from_secs(5),
    )
    .await
}

async fn wait_for_chain_committed(
    events: &mut EventStream<RollupManagerEvent>,
    expected_block_number: u64,
    expected_consolidated: bool,
    timeout: Duration,
) -> eyre::Result<()> {
    wait_for_event_predicate(
        events,
        |e| {
            if let RollupManagerEvent::ChainOrchestratorEvent(
                ChainOrchestratorEvent::L2ChainCommitted(block_info, _, consolidated),
            ) = e
            {
                return block_info.block_info.number == expected_block_number &&
                    expected_consolidated == consolidated;
            }

            false
        },
        timeout,
    )
    .await
}

async fn wait_for_event_predicate(
    event_stream: &mut EventStream<RollupManagerEvent>,
    mut predicate: impl FnMut(RollupManagerEvent) -> bool,
    timeout: Duration,
) -> eyre::Result<()> {
    let sleep = tokio::time::sleep(timeout);
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            maybe_event = event_stream.next() => {
                match maybe_event {
                    Some(e) if predicate(e.clone()) => {
                        tracing::debug!(target: "scroll::test", event = ?e, "Received event");
                        return Ok(());
                    }
                    Some(e) => {
                        tracing::debug!(target: "scroll::test", event = ?e, "Ignoring event");
                    }, // Ignore other events
                    None => return Err(eyre::eyre!("Event stream ended unexpectedly")),
                }
            }
            _ = &mut sleep => return Err(eyre::eyre!("Timeout while waiting for event")),
        }
    }
}

async fn wait_for_event_predicate_5s(
    event_stream: &mut EventStream<RollupManagerEvent>,
    predicate: impl FnMut(RollupManagerEvent) -> bool,
) -> eyre::Result<()> {
    wait_for_event_predicate(event_stream, predicate, Duration::from_secs(5)).await
}

async fn wait_for_event(
    event_stream: &mut EventStream<RollupManagerEvent>,
    event: RollupManagerEvent,
    timeout: Duration,
) -> eyre::Result<()> {
    wait_for_event_predicate(event_stream, |e| e == event, timeout).await
}

async fn wait_for_event_5s(
    event_stream: &mut EventStream<RollupManagerEvent>,
    event: RollupManagerEvent,
) -> eyre::Result<()> {
    wait_for_event(event_stream, event, Duration::from_secs(5)).await
}

/// Waits for n events to be emitted.
async fn wait_n_events(
    events: &mut EventStream<RollupManagerEvent>,
    mut matches: impl FnMut(RollupManagerEvent) -> bool,
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

async fn assert_latest_block_on_rpc_by_number(
    node: &NodeHelperType<
        ScrollRollupNode,
        BlockchainProvider<NodeTypesWithDBAdapter<ScrollRollupNode, TmpDB>>,
    >,
    block_number: u64,
) {
    eventually(
        Duration::from_secs(5),
        Duration::from_millis(100),
        "Waiting for latest block by number on node",
        || async {
            println!(
                "Latest block number: {}, hash: {}",
                latest_block(node).await.unwrap().header.number,
                latest_block(node).await.unwrap().header.hash_slow()
            );
            latest_block(node).await.unwrap().header.number == block_number
        },
    )
    .await;
}

async fn assert_latest_block_on_rpc_by_hash(
    node: &NodeHelperType<
        ScrollRollupNode,
        BlockchainProvider<NodeTypesWithDBAdapter<ScrollRollupNode, TmpDB>>,
    >,
    block_hash: B256,
) {
    eventually(
        Duration::from_secs(5),
        Duration::from_millis(100),
        "Waiting for latest block by hash on node",
        || async { latest_block(node).await.unwrap().header.hash_slow() == block_hash },
    )
    .await;
}
