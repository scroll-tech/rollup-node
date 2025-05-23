//! End-to-end tests for the rollup node.

use alloy_primitives::Signature;
use futures::StreamExt;
use reth_network::{NetworkConfigBuilder, PeersInfo};
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node::test_utils::build_bridge_node;
use scroll_network::NewBlockWithPeer;
use scroll_wire::ScrollWireConfig;
use tracing::trace;

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
    let (mut bridge_node, tasks, bridge_peer_id) =
        build_bridge_node(chain_spec.clone()).await.expect("Failed to setup nodes");

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
