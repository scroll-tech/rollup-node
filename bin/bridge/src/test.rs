
use alloy_primitives::B256;
use reth_e2e_test_utils::{node::NodeTestContext, NodeHelperType};
use reth_network::{NetworkConfigBuilder, PeersInfo};
use reth_network_peers::PeerId;
use reth_node_api::PayloadBuilderAttributes;
use reth_node_builder::{Node, NodeBuilder, NodeConfig, NodeHandle};
use reth_node_core::args::{DiscoveryArgs, NetworkArgs, RpcServerArgs};
use reth_provider::providers::BlockchainProvider;
use reth_rpc_server_types::RpcModuleSelection;
use reth_scroll_chainspec::{ScrollChainSpec, ScrollChainSpecBuilder};
use reth_scroll_engine_primitives::ScrollPayloadBuilderAttributes;
use reth_scroll_node::ScrollNode;
use reth_tasks::TaskManager;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_network::{BlockImport, SCROLL_MAINNET};
use scroll_wire::ScrollWireConfig;
use secp256k1::ecdsa::Signature;
use std::{
    collections::VecDeque,
    sync::Arc,
    task::{Context, Poll},
};
use tracing::trace;

/// A block import type that always returns a valid outcome.
#[derive(Debug, Default)]
pub struct ValidRethBlockImport {
    /// A buffer for storing the blocks that are received.
    blocks: VecDeque<(
        PeerId,
        reth_network::message::NewBlockMessage<reth_scroll_primitives::ScrollBlock>,
    )>,
    waker: Option<std::task::Waker>,
}

impl reth_network::import::BlockImport<reth_scroll_primitives::ScrollBlock>
    for ValidRethBlockImport
{
    fn on_new_block(
        &mut self,
        peer_id: PeerId,
        incoming_block: reth_network::message::NewBlockMessage<
            alloy_consensus::Block<reth_scroll_primitives::ScrollTransactionSigned>,
        >,
    ) {
        trace!(target: "network::import::ValidRethBlockImport", peer_id = %peer_id, block = ?incoming_block.block, "Received new block");
        self.blocks.push_back((peer_id, incoming_block));

        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        reth_network::import::BlockImportOutcome<
            alloy_consensus::Block<reth_scroll_primitives::ScrollTransactionSigned>,
        >,
    > {
        // If there are blocks in the buffer we return the first block.
        if let Some((peer, new_block)) = self.blocks.pop_front() {
            Poll::Ready(reth_network::import::BlockImportOutcome {
                peer,
                result: Ok(reth_network::import::BlockValidation::ValidBlock { block: new_block }),
            })
        } else {
            self.waker = Some(cx.waker().clone());
            Poll::Pending
        }
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
    let chain_spec = Arc::new(
        ScrollChainSpecBuilder::default()
            .chain(SCROLL_MAINNET.chain)
            .genesis(serde_json::from_str(include_str!("../assets/genesis.json")).unwrap())
            .darwin_v2_activated()
            .build(Default::default()),
    );

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
    let scroll_wire_config = ScrollWireConfig::new(false);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let scroll_network = scroll_network::NetworkManager::new(
        network_config,
        scroll_wire_config,
        TestBlockImport::new(tx),
    )
    .await;
    let scroll_network_handle = scroll_network.handle();

    // Spawn the scroll NetworkManager.
    tasks.executor().spawn(scroll_network);

    // Connect the scroll-wire node to the scroll NetworkManager.
    bridge_node.network.add_peer(scroll_network_handle.local_node_record()).await;
    bridge_node.network.next_session_established().await;

    // Create a standard NetworkManager to send blocks to the bridge node.
    let network_config =
        NetworkConfigBuilder::<reth_network::EthNetworkPrimitives>::with_rng_secret_key()
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
    let signature = Signature::from_compact(&[0u8; 64][..]).unwrap();
    let mut block: reth_primitives::Block<reth_primitives::TransactionSigned> =
        reth_primitives::Block::default();
    block.header.extra_data = signature.serialize_compact().into();
    let hash = block.hash_slow();
    let new_block = reth_eth_wire_types::NewBlock { block, ..Default::default() };
    network_handle.announce_block(new_block, hash);

    // wait for the block to be gossiped and received by the scroll-wire network manager from
    // the bridge.
    if let Some(msg) = rx.recv().await {
        match msg {
            scroll_wire::Event::NewBlock { block, signature: sig, peer_id } => {
                assert_eq!(block.hash_slow(), hash);
                assert_eq!(signature, sig);
                assert_eq!(peer_id, bridge_peer_id);
            }
            _ => panic!("Unexpected message"),
        }
    } else {
        panic!("No message received");
    }
}

#[derive(Debug)]
struct TestBlockImport {
    sender: tokio::sync::mpsc::UnboundedSender<scroll_wire::Event>,
}

impl TestBlockImport {
    pub fn new(sender: tokio::sync::mpsc::UnboundedSender<scroll_wire::Event>) -> Self {
        Self { sender }
    }
}

impl BlockImport for TestBlockImport {
    fn on_new_block(
        &mut self,
        peer_id: reth_network_peers::PeerId,
        block: reth_scroll_primitives::ScrollBlock,
        signature: secp256k1::ecdsa::Signature,
    ) {
        trace!(target: "bridge::import::TestBlockImport", peer_id = %peer_id, block = ?block, "Received new block from eth-wire protocol");
        let new_block = scroll_wire::Event::NewBlock { peer_id, block, signature };
        self.sender.send(new_block).unwrap();
    }

    fn poll(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<scroll_network::BlockImportOutcome> {
        std::task::Poll::Pending
    }
}

// HELPERS
// ---------------------------------------------------------------------------------------------

pub async fn build_bridge_node(
    chain_spec: Arc<ScrollChainSpec>,
) -> eyre::Result<(NodeHelperType<ScrollNode>, TaskManager, PeerId)> {
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
        .with_unused_ports()
        .with_rpc(
            RpcServerArgs::default()
                .with_unused_ports()
                .with_http()
                .with_http_api(RpcModuleSelection::All),
        )
        .set_dev(false);

    // Create the node for a bridge node that will bridge messages from the eth-wire protocol
    // to the scroll-wire protocol.
    let node = ScrollNode;
    let NodeHandle { node, node_exit_future: _ } = NodeBuilder::new(node_config.clone())
        .testing_node(exec.clone())
        .with_types_and_provider::<ScrollNode, BlockchainProvider<_>>()
        .with_components(node.components_builder().network(super::ScrollBridgeNetworkBuilder::new(
            Box::new(ValidRethBlockImport::default()),
        )))
        .with_add_ons(node.add_ons())
        .launch()
        .await?;
    let peer_id = *node.network.peer_id();
    let node = NodeTestContext::new(node, scroll_payload_attributes).await?;

    Ok((node, tasks, peer_id))
}

/// Helper function to create a new eth payload attributes
fn scroll_payload_attributes(_timestamp: u64) -> ScrollPayloadBuilderAttributes {
    let attributes = ScrollPayloadAttributes::default();
    ScrollPayloadBuilderAttributes::try_new(B256::ZERO, attributes, 0).unwrap()
}
