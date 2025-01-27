//! Scroll binary
use reth_node_builder::{
    components::NodeComponentsBuilder,
    rpc::{EngineValidatorAddOn, RethRpcAddOns},
    EngineNodeLauncher, FullNodeTypesAdapter, Node, NodeAdapter, NodeBuilder, NodeComponents,
    NodeConfig, NodeHandle, NodeTypesWithDBAdapter, NodeTypesWithEngine, PayloadAttributesBuilder,
    PayloadTypes,
};
use reth_node_core::args::{DiscoveryArgs, NetworkArgs, RpcServerArgs};
use reth_provider::providers::{BlockchainProvider, NodeTypesForProvider, NodeTypesForTree};
use std::sync::Arc;
use tracing::{span, Level};

mod network;
use network::ScrollBridgeNetworkBuilder;

mod import;
use import::{BridgeBlockImport, ValidBlockImport};

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

#[cfg(all(feature = "scroll", not(feature = "optimism")))]
fn main() {
    use clap::Parser;
    use reth_node_builder::{engine_tree_config::TreeConfig, EngineNodeLauncher};
    use reth_provider::providers::BlockchainProvider;
    use reth_scroll_cli::{Cli, ScrollChainSpecParser, ScrollRollupArgs};
    use reth_scroll_node::{ScrollAddOns, ScrollNode};
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if let Err(err) = Cli::<ScrollChainSpecParser, ScrollRollupArgs>::parse()
        .run::<_, _, ScrollNode>(|builder, _| async move {
            let engine_tree_config = TreeConfig::default()
                .with_persistence_threshold(builder.config().engine.persistence_threshold)
                .with_memory_block_buffer_target(
                    builder.config().engine.memory_block_buffer_target,
                );
            let handle = builder
                .with_types_and_provider::<ScrollNode, BlockchainProvider<_>>()
                // Override the network builder with the `ScrollBridgeNetworkBuilder`
                .with_components(ScrollNode::components().network(ScrollBridgeNetworkBuilder))
                .with_add_ons(ScrollAddOns::default())
                .launch_with_fn(|builder| {
                    let launcher = EngineNodeLauncher::new(
                        builder.task_executor().clone(),
                        builder.config().datadir(),
                        engine_tree_config,
                    );
                    builder.launch_with(launcher)
                })
                .await?;

            handle.node_exit_future.await
        })
    {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}

#[cfg(any(feature = "optimism", not(feature = "scroll")))]
fn main() {
    eprintln!("Scroll feature is not enabled");
    std::process::exit(1);
}

#[cfg(test)]
mod test {
    use alloy_primitives::{Address, B256};
    use alloy_rpc_types_engine::PayloadAttributes;
    use reth_e2e_test_utils::{node::NodeTestContext, wallet::Wallet, NodeHelperType};
    use reth_network::{NetworkConfigBuilder, NetworkEventListenerProvider, PeersInfo};
    use reth_network_peers::PeerId;
    use reth_node_builder::{rpc::ExtendRpcModules, Node, NodeBuilder, NodeConfig, NodeHandle};
    use reth_node_core::args::{DiscoveryArgs, NetworkArgs, RpcServerArgs};
    use reth_payload_builder::EthPayloadBuilderAttributes;
    use reth_provider::providers::BlockchainProvider;
    use reth_rpc_server_types::RpcModuleSelection;
    use reth_scroll_chainspec::{ScrollChainSpec, ScrollChainSpecBuilder};
    use reth_scroll_node::{ScrollAddOns, ScrollNode};
    use reth_tasks::TaskManager;
    use scroll_network::{BlockImport, BlockImportOutcome, BlockValidation, SCROLL_MAINNET};
    use scroll_wire::{NewBlock, ScrollWireConfig};
    use secp256k1::ecdsa::Signature;
    use std::{collections::VecDeque, sync::Arc};
    use tracing::trace;

    /// We test the bridge from the eth-wire protocol to the scroll-wire protocol.
    ///
    /// This test will launch three nodes:
    /// - Node 1: The bridge node that will bridge messages from the eth-wire protocol to the
    ///   scroll-wire protocol.
    /// - Node 2: A scroll-wire node that will receive the bridged messages.
    /// - Node 3: A standard node that will send messages to the bridge node on the eth-wire
    ///   protocol.
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
        let (mut bridge_node, tasks, _wallet) =
            build_bridge_node(chain_spec.clone()).await.expect("Failed to setup nodes");

        // Instantiate the scroll NetworkManager.
        let network_config =
            NetworkConfigBuilder::<reth_network::EthNetworkPrimitives>::with_rng_secret_key()
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
                    // assert_eq!(&peer_id, bridge_node.network.handle().pee);
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
            block: reth_primitives::Block,
            signature: secp256k1::ecdsa::Signature,
        ) {
            trace!(target: "bridge::import::TestBlockImport", peer_id = %peer_id, block = ?block, "Received new block from eth-wire protocol");
            let new_block = scroll_wire::Event::NewBlock { peer_id, block, signature };
            self.sender.send(new_block).unwrap();
        }

        fn poll(
            &mut self,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<scroll_network::BlockImportOutcome> {
            std::task::Poll::Pending
        }
    }

    // HELPERS
    // ---------------------------------------------------------------------------------------------

    pub async fn build_bridge_node(
        chain_spec: Arc<ScrollChainSpec>,
    ) -> eyre::Result<(NodeHelperType<ScrollNode>, TaskManager, Wallet)> {
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
            .with_components(node.components_builder().network(super::ScrollBridgeNetworkBuilder))
            .with_add_ons(node.add_ons())
            .launch()
            .await?;
        let node = NodeTestContext::new(node, eth_payload_attributes).await?;

        Ok((node, tasks, Wallet::default().with_chain_id(chain_spec.chain().into())))
    }

    /// Helper function to create a new eth payload attributes
    fn eth_payload_attributes(timestamp: u64) -> EthPayloadBuilderAttributes {
        let attributes = PayloadAttributes {
            timestamp,
            prev_randao: B256::ZERO,
            suggested_fee_recipient: Address::ZERO,
            withdrawals: Some(vec![]),
            parent_beacon_block_root: Some(B256::ZERO),
        };
        EthPayloadBuilderAttributes::new(B256::ZERO, attributes)
    }
}
