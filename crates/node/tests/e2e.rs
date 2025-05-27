//! End-to-end tests for the rollup node.

use alloy_primitives::{Address, Signature, U256};
use futures::StreamExt;
use reth_network::{NetworkConfigBuilder, PeersInfo};
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node::{
    test_utils::{default_test_scroll_rollup_node_config, generate_tx, setup_engine},
    BeaconProviderArgs, DatabaseArgs, EngineDriverArgs, L1ProviderArgs,
    NetworkArgs as ScrollNetworkArgs, ScrollRollupNodeConfig, SequencerArgs,
};
use rollup_node_manager::{RollupManagerEvent, RollupManagerHandle};
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_network::NewBlockWithPeer;
use scroll_wire::ScrollWireConfig;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tracing::trace;

#[tokio::test]
async fn can_bridge_l1_messages() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll mainnet with Euclid v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let node_args = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs::default(),
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
        assert_eq!(block.body.transactions[0].transaction.l1_message().unwrap(), &l1_message,);
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
        },
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs::default(),
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

    // Create the chain spec for scroll dev with Euclid v2 activated and a test genesis.
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
