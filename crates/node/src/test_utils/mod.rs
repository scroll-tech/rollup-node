//! Test utilities for the Scroll rollup node.
//!
//! This module provides a high-level test framework for creating and managing
//! test nodes, building blocks, managing L1 interactions, and asserting on events.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use rollup_node::test_utils::TestFixture;
//!
//! #[tokio::test]
//! async fn test_basic_block_production() -> eyre::Result<()> {
//!     let mut fixture = TestFixture::sequencer().build().await?;
//!
//!     // Inject a transaction
//!     let tx_hash = fixture.inject_transfer().await?;
//!
//!     // Build a block
//!     let block = fixture.build_block()
//!         .expect_tx(tx_hash)
//!         .await_block()
//!         .await?;
//!
//!     // Get the current block
//!     let current_block = fixture.get_sequencer_block().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Event Assertions
//!
//! The framework provides powerful event assertion capabilities:
//!
//! ```rust,ignore
//! // Wait for events on a single node
//! fixture.expect_event_on(1).chain_extended().await?;
//!
//! // Wait for the same event on multiple nodes
//! fixture.expect_event_on_followers().new_block_received().await?;
//!
//! // Wait for events on all nodes (including sequencer)
//! fixture.expect_event_on_all_nodes().chain_extended().await?;
//!
//! // Custom event predicates - just check if event matches
//! fixture.expect_event()
//!     .where_event(|e| matches!(e, ChainOrchestratorEvent::BlockSequenced(_)))
//!     .await?;
//!
//! // Extract values from events
//! let block_numbers = fixture.expect_event_on_nodes(vec![1, 2])
//!     .extract(|e| {
//!         if let ChainOrchestratorEvent::NewL1Block(num) = e {
//!             Some(*num)
//!         } else {
//!             None
//!         }
//!     })
//!     .await?;
//! ```

// Module declarations
pub mod block_builder;
pub mod database;
pub mod event_utils;
pub mod fixture;
pub mod l1_helpers;
pub mod network_helpers;
pub mod reboot;
pub mod tx_helpers;

// Re-export main types for convenience
pub use database::{DatabaseHelper, DatabaseOperations};
pub use event_utils::{EventAssertions, EventWaiter};
pub use fixture::{NodeHandle, TestFixture, TestFixtureBuilder};
pub use network_helpers::{
    NetworkHelper, NetworkHelperProvider, ReputationChecker, ReputationChecks,
};

// Legacy utilities - keep existing functions for backward compatibility
use crate::{
    test_utils::fixture::ScrollNodeTestComponents, BlobProviderArgs, ChainOrchestratorArgs,
    ConsensusArgs, EngineDriverArgs, L1ProviderArgs, RollupNodeDatabaseArgs, RollupNodeNetworkArgs,
    RpcArgs, ScrollRollupNode, ScrollRollupNodeConfig, SequencerArgs, TestArgs,
};
use alloy_primitives::Bytes;
use reth_chainspec::EthChainSpec;
use reth_db::test_utils::create_test_rw_db_with_path;
use reth_e2e_test_utils::{
    node::NodeTestContext, transaction::TransactionTestContext, wallet::Wallet, Adapter,
    TmpNodeAddOnsHandle, TmpNodeEthApi,
};
use reth_engine_local::LocalPayloadAttributesBuilder;
use reth_node_builder::{
    rpc::RpcHandleProvider, EngineNodeLauncher, Node, NodeBuilder, NodeConfig,
    NodeHandle as RethNodeHandle, NodeTypes, PayloadAttributesBuilder, PayloadTypes, TreeConfig,
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
///
/// This is the legacy setup function that's used by existing tests.
/// For new tests, consider using the `TestFixture` API instead.
pub async fn setup_engine(
    mut scroll_node_config: ScrollRollupNodeConfig,
    num_nodes: usize,
    chain_spec: Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>,
    is_dev: bool,
    no_local_transactions_propagation: bool,
    reboot_info: Option<(usize, Arc<reth_db::test_utils::TempDatabase<reth_db::DatabaseEnv>>)>,
) -> eyre::Result<(
    Vec<ScrollNodeTestComponents>,
    Vec<Arc<reth_db::test_utils::TempDatabase<reth_db::DatabaseEnv>>>,
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
    let network_config = NetworkArgs {
        discovery: DiscoveryArgs { disable_discovery: true, ..DiscoveryArgs::default() },
        ..NetworkArgs::default()
    };

    // Create nodes and peer them
    let mut nodes: Vec<ScrollNodeTestComponents> = Vec::with_capacity(num_nodes);
    let mut dbs: Vec<Arc<reth_db::test_utils::TempDatabase<reth_db::DatabaseEnv>>> = Vec::new();

    for idx in 0..num_nodes {
        // Determine the actual node index (for reboot use provided index, otherwise use idx)
        let node_index = reboot_info.as_ref().map(|(node_idx, _)| *node_idx).unwrap_or(idx);

        // Disable sequencer for all nodes except index 0
        if node_index != 0 {
            scroll_node_config.sequencer_args.sequencer_enabled = false;
        }

        // Configure node with the test data directory
        let mut node_config = NodeConfig::new(chain_spec.clone())
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

        // Check if we already have provided a database for a node (reboot scenario)
        let db = if let Some((_, provided_db)) = &reboot_info {
            // Reuse existing database for reboot
            let db_path = provided_db.path();
            let test_data_dir = db_path.parent().expect("db path should have a parent directory");

            // Set the datadir in node_config to reuse the same directory
            node_config.datadir.datadir =
                reth_node_core::dirs::MaybePlatformPath::from(test_data_dir.to_path_buf());

            tracing::info!(
                "Reusing existing database for node {} at {:?}",
                node_index,
                test_data_dir
            );
            provided_db.clone()
        } else {
            // Create a unique persistent test directory for both Reth and Scroll databases
            // Using process ID and node index to ensure uniqueness
            let test_data_dir = std::env::temp_dir().join(format!(
                "scroll-test-{}-node-{}-{}",
                std::process::id(),
                node_index,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&test_data_dir).expect("failed to create test data directory");

            // Set the datadir in node_config (critical for proper initialization)
            node_config.datadir.datadir =
                reth_node_core::dirs::MaybePlatformPath::from(test_data_dir.clone());

            // Create Reth database in the test directory's db subdirectory
            let new_db = create_test_rw_db_with_path(node_config.datadir().db());

            tracing::info!("Created new database for node {} at {:?}", node_index, test_data_dir);
            dbs.push(new_db.clone());
            new_db
        };

        let span = span!(Level::INFO, "node", node_index);
        let _enter = span.enter();
        let task_manager = TaskManager::current();
        let testing_node = NodeBuilder::new(node_config.clone())
            .with_database(db.clone())
            .with_launch_context(task_manager.executor());
        let testing_config = testing_node.config().clone();
        let node = ScrollRollupNode::new(scroll_node_config.clone(), testing_config).await;
        let RethNodeHandle { node, node_exit_future } = testing_node
            .with_types_and_provider::<ScrollRollupNode, BlockchainProvider<_>>()
            .with_components(node.components_builder())
            .with_add_ons(node.add_ons())
            .launch_with_fn(|builder| {
                let tree_config = TreeConfig::default()
                    .with_always_process_payload_attributes_on_canonical_head(true)
                    .with_unwind_canonical_header(true)
                    .with_persistence_threshold(0);
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

        let node = ScrollNodeTestComponents::new(node, task_manager, node_exit_future).await;

        nodes.push(node);
        // Note: db is already added to dbs in the creation logic above
    }

    Ok((nodes, dbs, Wallet::default().with_chain_id(chain_spec.chain().into())))
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
        test_args: TestArgs { test: true, skip_l1_synced: false },
        network_args: RollupNodeNetworkArgs::default(),
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
        gas_price_oracle_args: crate::RollupNodeGasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
        rpc_args: RpcArgs { basic_enabled: true, admin_enabled: true },
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
        test_args: TestArgs { test: true, skip_l1_synced: false },
        network_args: RollupNodeNetworkArgs::default(),
        database_args: RollupNodeDatabaseArgs {
            rn_db_path: Some(PathBuf::from("sqlite::memory:")),
        },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs { sync_at_startup: true },
        chain_orchestrator_args: ChainOrchestratorArgs {
            optimistic_sync_trigger: 100,
            chain_buffer_size: 100,
        },
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            auto_start: false,
            block_time: 100,
            payload_building_duration: 40,
            fee_recipient: Default::default(),
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            allow_empty_blocks: true,
            max_l1_messages: None,
        },
        blob_provider_args: BlobProviderArgs { mock: true, ..Default::default() },
        signer_args: Default::default(),
        gas_price_oracle_args: crate::RollupNodeGasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
        rpc_args: RpcArgs { basic_enabled: true, admin_enabled: true },
    }
}
