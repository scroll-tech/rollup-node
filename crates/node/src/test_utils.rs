//! This crate contains utilities for running end-to-end tests for the scroll reth node.

use super::{
    BeaconProviderArgs, DatabaseArgs, EngineDriverArgs, L1ProviderArgs, ScrollRollupNode,
    ScrollRollupNodeConfig, SequencerArgs,
};
use alloy_primitives::Bytes;
use reth_chainspec::EthChainSpec;
use reth_e2e_test_utils::{
    node::NodeTestContext, transaction::TransactionTestContext, wallet::Wallet, Adapter,
    NodeHelperType, TmpDB, TmpNodeAddOnsHandle, TmpNodeEthApi,
};
use reth_engine_local::LocalPayloadAttributesBuilder;
use reth_node_builder::{
    rpc::RpcHandleProvider, EngineNodeLauncher, Node, NodeBuilder, NodeConfig, NodeHandle,
    NodeTypes, NodeTypesWithDBAdapter, PayloadAttributesBuilder, PayloadTypes,
};
use reth_node_core::args::{DiscoveryArgs, NetworkArgs, RpcServerArgs};
use reth_provider::providers::BlockchainProvider;
use reth_rpc_server_types::RpcModuleSelection;
use reth_tasks::TaskManager;
use rollup_node_providers::BlobSource;
use rollup_node_sequencer::L1MessageInclusionMode;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tracing::{span, Level};

/// Creates the initial setup with `num_nodes` started and interconnected.
pub async fn setup_engine(
    mut scroll_node_config: ScrollRollupNodeConfig,
    num_nodes: usize,
    chain_spec: Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>,
    is_dev: bool,
) -> eyre::Result<(
    Vec<
        NodeHelperType<
            ScrollRollupNode,
            BlockchainProvider<NodeTypesWithDBAdapter<ScrollRollupNode, TmpDB>>,
        >,
    >,
    TaskManager,
    Wallet,
)>
where
    LocalPayloadAttributesBuilder<<ScrollRollupNode as NodeTypes>::ChainSpec>:
        PayloadAttributesBuilder<
            <<ScrollRollupNode as NodeTypes>::Payload as PayloadTypes>::PayloadAttributes,
        >,
    TmpNodeAddOnsHandle<ScrollRollupNode>:
        RpcHandleProvider<Adapter<ScrollRollupNode>, TmpNodeEthApi<ScrollRollupNode>>,
{
    let tasks = TaskManager::current();
    let exec = tasks.executor();

    let network_config = NetworkArgs {
        discovery: DiscoveryArgs { disable_discovery: true, ..DiscoveryArgs::default() },
        ..NetworkArgs::default()
    };

    // Create nodes and peer them
    let mut nodes: Vec<NodeTestContext<_, _>> = Vec::with_capacity(num_nodes);

    for idx in 0..num_nodes {
        // disable sequencer nodes after the first one
        if idx != 0 {
            scroll_node_config.sequencer_args.sequencer_enabled = false;
        }
        let node_config = NodeConfig::new(chain_spec.clone())
            .with_network(network_config.clone())
            .with_unused_ports()
            .with_rpc(
                RpcServerArgs::default()
                    .with_unused_ports()
                    .with_http()
                    .with_http_api(RpcModuleSelection::All),
            )
            .set_dev(is_dev);

        let span = span!(Level::INFO, "node", idx);
        let _enter = span.enter();
        let node = ScrollRollupNode::new(scroll_node_config.clone());
        let NodeHandle { node, node_exit_future: _ } = NodeBuilder::new(node_config.clone())
            .testing_node(exec.clone())
            .with_types_and_provider::<ScrollRollupNode, BlockchainProvider<_>>()
            .with_components(node.components_builder())
            .with_add_ons(node.add_ons())
            .launch_with_fn(|builder| {
                let launcher = EngineNodeLauncher::new(
                    builder.task_executor().clone(),
                    builder.config().datadir(),
                    Default::default(),
                );
                builder.launch_with(launcher)
            })
            .await?;

        let mut node =
            NodeTestContext::new(node, |_| panic!("should not build payloads using this method"))
                .await?;

        let genesis = node.block_hash(0);
        node.update_forkchoice(genesis, genesis).await?;

        // Connect each node in a chain.
        if let Some(previous_node) = nodes.last_mut() {
            previous_node.connect(&mut node).await;
        }

        // Connect last node with the first if there are more than two
        if idx + 1 == num_nodes && num_nodes > 2 {
            if let Some(first_node) = nodes.first_mut() {
                node.connect(first_node).await;
            }
        }

        nodes.push(node);
    }

    Ok((nodes, tasks, Wallet::default().with_chain_id(chain_spec.chain().into())))
}

/// Generate a transfer transaction with the given wallet.
pub async fn generate_tx(wallet: Arc<Mutex<Wallet>>) -> Bytes {
    let mut wallet = wallet.lock().await;
    let tx_fut = TransactionTestContext::transfer_tx_nonce_bytes(
        wallet.chain_id,
        wallet.inner.clone(),
        wallet.inner_nonce,
    );
    wallet.inner_nonce += 1;
    tx_fut.await
}

/// Returns a default [`ScrollRollupNodeConfig`] preconfigured for testing.
pub fn default_test_scroll_rollup_node_config() -> ScrollRollupNodeConfig {
    ScrollRollupNodeConfig {
        test: true,
        network_args: crate::args::NetworkArgs::default(),
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs { en_sync_trigger: 100, sync_at_startup: true },
        sequencer_args: SequencerArgs { payload_building_duration: 1000, ..Default::default() },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
    }
}

/// Returns a default [`ScrollRollupNodeConfig`] preconfigured for testing with sequencer.
pub fn default_sequencer_test_scroll_rollup_node_config() -> ScrollRollupNodeConfig {
    ScrollRollupNodeConfig {
        test: true,
        network_args: crate::args::NetworkArgs::default(),
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs { en_sync_trigger: 100, sync_at_startup: true },
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 50,
            payload_building_duration: 40,
            max_l1_messages_per_block: 0,
            fee_recipient: Default::default(),
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: Default::default(),
    }
}
