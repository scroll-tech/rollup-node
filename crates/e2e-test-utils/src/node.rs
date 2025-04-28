use alloy_primitives::B256;
use reth_db::{test_utils::TempDatabase, DatabaseEnv};
use reth_e2e_test_utils::{node::NodeTestContext, NodeHelperType};
use reth_network_peers::PeerId;
use reth_node_builder::{
    components::{NetworkBuilder, PoolBuilder},
    FullNodeTypesAdapter, Node, NodeBuilder, NodeHandle, NodeTypesWithDBAdapter,
    PayloadBuilderAttributes,
};
use reth_node_core::{
    args::{NetworkArgs, RpcServerArgs},
    node_config::NodeConfig,
};
use reth_provider::providers::BlockchainProvider;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_node::{
    ScrollNetworkPrimitives, ScrollNode, ScrollPayloadBuilderAttributes, ScrollPoolBuilder,
};
use reth_tasks::TaskManager;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use std::sync::Arc;

type TestDB = Arc<TempDatabase<DatabaseEnv>>;
type TestNode = FullNodeTypesAdapter<
    ScrollNode,
    TestDB,
    BlockchainProvider<NodeTypesWithDBAdapter<ScrollNode, TestDB>>,
>;

/// Test helper to build a Reth node, along with any provided additional components.
pub async fn build_node<NetworkB>(
    chain_spec: Arc<ScrollChainSpec>,
    net_args: NetworkArgs,
    rpc_args: RpcServerArgs,
    network_extension: NetworkB,
) -> eyre::Result<(NodeHelperType<ScrollNode>, TaskManager, PeerId)>
where
    NetworkB: NetworkBuilder<
        TestNode,
        <ScrollPoolBuilder as PoolBuilder<TestNode>>::Pool,
        Primitives = ScrollNetworkPrimitives,
    >,
{
    // Create a [`TaskManager`] to manage the tasks.
    let tasks = TaskManager::current();
    let exec = tasks.executor();

    // Create the node config
    let node_config = NodeConfig::new(chain_spec.clone())
        .with_network(net_args)
        .with_rpc(rpc_args)
        .set_dev(false);

    let node = ScrollNode;
    let NodeHandle { node, node_exit_future: _ } = NodeBuilder::new(node_config.clone())
        .testing_node(exec.clone())
        .with_types_and_provider::<ScrollNode, BlockchainProvider<_>>()
        .with_components(node.components_builder().network(network_extension))
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
