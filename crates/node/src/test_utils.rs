//! Test utils to spawn test nodes.

use crate::{
    BeaconProviderArgs, L1ProviderArgs, L2ProviderArgs, ScrollRollupNode, ScrollRollupNodeConfig,
    SequencerArgs,
};

use alloy_primitives::B256;
use alloy_rpc_types_engine::PayloadAttributes;
use reth_e2e_test_utils::{node::NodeTestContext, NodeHelperType};
use reth_network_peers::PeerId;
use reth_node_api::PayloadBuilderAttributes;
use reth_node_builder::{NodeBuilder, NodeHandle};
use reth_node_core::{
    args::{DiscoveryArgs, NetworkArgs, RpcServerArgs},
    node_config::NodeConfig,
};
use reth_rpc_server_types::RpcModuleSelection;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_engine_primitives::ScrollPayloadBuilderAttributes;
use reth_tasks::TaskManager;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use std::{path::PathBuf, sync::Arc};

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
        .with_unused_ports()
        .set_dev(false);

    // Create the node for a bridge node that will bridge messages from the eth-wire protocol
    // to the scroll-wire protocol.
    let node_args = ScrollRollupNodeConfig {
        test: true,
        network_args: crate::args::NetworkArgs {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
        },
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
fn scroll_payload_attributes(timestamp: u64) -> ScrollPayloadBuilderAttributes {
    let attributes = ScrollPayloadAttributes {
        payload_attributes: PayloadAttributes { timestamp, ..Default::default() },
        ..Default::default()
    };
    ScrollPayloadBuilderAttributes::try_new(B256::ZERO, attributes, 0).unwrap()
}
