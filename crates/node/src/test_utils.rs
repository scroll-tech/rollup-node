//! This crate contains utilities for running end-to-end tests for the scroll reth node.

use crate::{ConsensusArgs, RollupNodeGasPriceOracleArgs};

use super::{
    BlobProviderArgs, ChainOrchestratorArgs, RollupNodeDatabaseArgs, EngineDriverArgs, L1ProviderArgs,
    RpcArgs, ScrollRollupNode, ScrollRollupNodeConfig, SequencerArgs,
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
    NodeTypes, NodeTypesWithDBAdapter, PayloadAttributesBuilder, PayloadTypes, TreeConfig,
};
use reth_node_core::args::{DiscoveryArgs, NetworkArgs, RpcServerArgs, TxPoolArgs};
use reth_provider::providers::BlockchainProvider;
use reth_rpc_server_types::RpcModuleSelection;
use reth_tasks::TaskManager;
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
    no_local_transactions_propagation: bool,
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
            .set_dev(is_dev)
            .with_txpool(TxPoolArgs { no_local_transactions_propagation, ..Default::default() });

        let span = span!(Level::INFO, "node", idx);
        let _enter = span.enter();
        let testing_node = NodeBuilder::new(node_config.clone()).testing_node(exec.clone());
        let testing_config = testing_node.config().clone();
        let node = ScrollRollupNode::new(scroll_node_config.clone(), testing_config).await;
        let NodeHandle { node, node_exit_future: _ } = testing_node
            .with_types_and_provider::<ScrollRollupNode, BlockchainProvider<_>>()
            .with_components(node.components_builder())
            .with_add_ons(node.add_ons())
            .launch_with_fn(|builder| {
                let tree_config = TreeConfig::default()
                    .with_always_process_payload_attributes_on_canonical_head(true);
                let launcher = EngineNodeLauncher::new(
                    builder.task_executor().clone(),
                    builder.config().datadir(),
                    tree_config,
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
        network_args: crate::args::RollupNodeNetworkArgs::default(),
        database_args: RollupNodeDatabaseArgs::default(),
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs { sync_at_startup: true },
        chain_orchestrator_args: ChainOrchestratorArgs {
            optimistic_sync_trigger: 100,
            chain_buffer_size: 100,
        },
        sequencer_args: SequencerArgs {
            payload_building_duration: 1000,
            allow_empty_blocks: true,
            ..Default::default()
        },
        blob_provider_args: BlobProviderArgs { mock: true, ..Default::default() },
        signer_args: Default::default(),
        gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
        rpc_args: RpcArgs { enabled: true },
    }
}

/// Returns a default [`ScrollRollupNodeConfig`] preconfigured for testing with sequencer.
/// It sets `sequencer_args.block_time = 0` so that no blocks are produced automatically.
/// To produce blocks the `build_block` method needs to be invoked.
/// This is so that block production and test scenarios remain predictable.
///
/// In case this behavior is not wanted, `block_time` can be adjusted to any value > 0 after
/// obtaining the config so that the sequencer node will produce blocks automatically in this
/// interval.
pub fn default_sequencer_test_scroll_rollup_node_config() -> ScrollRollupNodeConfig {
    ScrollRollupNodeConfig {
        test: true,
        network_args: crate::args::RollupNodeNetworkArgs::default(),
        database_args: RollupNodeDatabaseArgs { rn_db_path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs { sync_at_startup: true },
        chain_orchestrator_args: ChainOrchestratorArgs {
            optimistic_sync_trigger: 100,
            chain_buffer_size: 100,
        },
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            auto_start: true,
            block_time: 0,
            payload_building_duration: 40,
            fee_recipient: Default::default(),
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            allow_empty_blocks: true,
        },
        blob_provider_args: BlobProviderArgs { mock: true, ..Default::default() },
        signer_args: Default::default(),
        gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
        rpc_args: RpcArgs { enabled: true },
    }
}
