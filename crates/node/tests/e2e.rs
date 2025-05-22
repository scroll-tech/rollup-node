//! End-to-end tests for the rollup node.

use alloy_primitives::{Address, B256, U256};
use futures::StreamExt;
use reth_e2e_test_utils::{node::NodeTestContext, NodeHelperType};
use reth_network::{NetworkConfigBuilder, PeersInfo};
use reth_network_peers::PeerId;
use reth_node_api::PayloadBuilderAttributes;
use reth_node_builder::{NodeBuilder, NodeConfig, NodeHandle};
use reth_node_core::args::{DiscoveryArgs, NetworkArgs, RpcServerArgs};
use reth_rpc_server_types::RpcModuleSelection;
use reth_scroll_chainspec::{ScrollChainSpec, SCROLL_DEV};
use reth_scroll_engine_primitives::ScrollPayloadBuilderAttributes;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_tasks::TaskManager;
use rollup_node::{
    test_utils::{generate_tx, setup_engine},
    BeaconProviderArgs, L1ProviderArgs, L2ProviderArgs, NetworkArgs as ScrollNetworkArgs,
    ScrollRollupNode, ScrollRollupNodeConfig, SequencerArgs,
};
use rollup_node_manager::{RollupManagerEvent, RollupManagerHandle};
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_network::{NewBlockWithPeer, SCROLL_MAINNET};
use scroll_wire::ScrollWireConfig;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tracing::trace;

#[tokio::test]
async fn can_bridge_l1_messages() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create the chain spec for scroll mainnet with Darwin v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let node_args = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
        },
        optimistic_sync: false,
        database_path: Some(PathBuf::from("sqlite::memory:")),
        l1_provider_args: L1ProviderArgs::default(),
        engine_api_url: None,
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs::default(),
        l2_provider_args: L2ProviderArgs::default(),
    };
    let (mut nodes, _tasks, _wallet) = setup_engine(node_args, 1, chain_spec, false).await.unwrap();
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
        optimistic_sync: false,
        database_path: Some(PathBuf::from("sqlite::memory:")),
        l1_provider_args: L1ProviderArgs::default(),
        engine_api_url: None,
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs::default(),
        l2_provider_args: L2ProviderArgs::default(),
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

    // Create the chain spec for scroll mainnet with Darwin v2 activated and a test genesis.
    let chain_spec = (*SCROLL_MAINNET).clone();

    // Setup the bridge node and a standard node.
    let (mut bridge_node, tasks, bridge_peer_id) =
        build_bridge_node(chain_spec.clone()).await.expect("Failed to setup nodes");

    // Instantiate the scroll NetworkManager.
    let network_config =
        NetworkConfigBuilder::<reth_scroll_node::ScrollNetworkPrimitives>::with_rng_secret_key()
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

    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    // Send a block from the standard NetworkManager to the bridge node.
    let block_1: reth_scroll_primitives::ScrollBlock =
        serde_json::from_str(include_str!("../assets/block_1.json")).unwrap();
    let block_1_hash = block_1.hash_slow();
    let new_block_1 = reth_eth_wire_types::NewBlock { block: block_1, ..Default::default() };

    trace!("Announcing block to eth-wire network");
    network_handle.announce_block(new_block_1, block_1_hash);

    // Assert block received from the bridge node on the scroll wire protocol is correct
    if let Some(scroll_network::NetworkManagerEvent::NewBlock(NewBlockWithPeer {
        peer_id,
        block,
        signature: _,
    })) = scroll_network.next().await
    {
        assert_eq!(peer_id, bridge_peer_id);
        assert_eq!(block.hash_slow(), block_1_hash);
    } else {
        panic!("Failed to receive block from scroll-wire network");
    }
}

// HELPERS
// ---------------------------------------------------------------------------------------------
/// Helper function to create a new bridge node that will bridge messages from the eth-wire
pub async fn build_bridge_node(
    chain_spec: Arc<ScrollChainSpec>,
) -> eyre::Result<(NodeHelperType<ScrollRollupNode>, TaskManager, PeerId)> {
    // Create a [`TaskManager`] to manage the tasks.
    let tasks = TaskManager::current();
    let exec = tasks.executor();

    // Define the network configuration with discovery disabled.
    let network_config = NetworkArgs {
        discovery: DiscoveryArgs { disable_discovery: true, ..DiscoveryArgs::default() },
        ..NetworkArgs::default()
    };

    // Create the node config
    let node_config = NodeConfig::new(chain_spec.clone())
        .with_network(network_config.clone())
        .with_rpc(RpcServerArgs::default().with_http().with_http_api(RpcModuleSelection::All))
        .set_dev(false);

    // Create the node for a bridge node that will bridge messages from the eth-wire protocol
    // to the scroll-wire protocol.
    let node_args = ScrollRollupNodeConfig {
        test: true,
        network_args: ScrollNetworkArgs {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
        },
        optimistic_sync: false,
        database_path: Some(PathBuf::from("sqlite::memory:")),
        l1_provider_args: L1ProviderArgs::default(),
        engine_api_url: None,
        sequencer_args: SequencerArgs { sequencer_enabled: false, ..SequencerArgs::default() },
        beacon_provider_args: BeaconProviderArgs::default(),
        l2_provider_args: L2ProviderArgs::default(),
    };
    let node = ScrollRollupNode::new(node_args);
    let NodeHandle { node, node_exit_future: _ } =
        NodeBuilder::new(node_config.clone()).testing_node(exec.clone()).launch_node(node).await?;
    let peer_id = *node.network.peer_id();
    let node = NodeTestContext::new(node, scroll_payload_attributes).await?;

    Ok((node, tasks, peer_id))
}

/// Helper function to create a new eth payload attributes
fn scroll_payload_attributes(_timestamp: u64) -> ScrollPayloadBuilderAttributes {
    let attributes = ScrollPayloadAttributes::default();
    ScrollPayloadBuilderAttributes::try_new(B256::ZERO, attributes, 0).unwrap()
}
