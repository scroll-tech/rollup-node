//! Core test fixture for setting up and managing test nodes.

use super::{
    block_builder::BlockBuilder, config::TestConfigBuilder, l1_helpers::L1Helper, setup_engine,
    tx_helpers::TxHelper,
};
use crate::ScrollRollupNode;
use alloy_primitives::{Address, BlockHash};
use jsonrpsee::{
    core::middleware::layer::RpcLogger,
    http_client::{transport::HttpBackend, HttpClient, RpcService},
};
use reth_chainspec::EthChainSpec;
use reth_e2e_test_utils::{wallet::Wallet, NodeHelperType, TmpDB};
use reth_eth_wire_types::BasicNetworkPrimitives;
use reth_network::NetworkHandle;
use reth_node_builder::NodeTypes;
use reth_node_types::NodeTypesWithDBAdapter;
use reth_provider::providers::BlockchainProvider;
use reth_rpc_api::EngineApiClient;
use reth_rpc_layer::AuthClientService;
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_engine_primitives::ScrollEngineTypes;
use reth_scroll_primitives::ScrollPrimitives;
use reth_tasks::TaskManager;
use reth_tokio_util::EventStream;
use rollup_node_chain_orchestrator::{ChainOrchestratorEvent, ChainOrchestratorHandle};
use rollup_node_primitives::BlockInfo;
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::ScrollPooledTransaction;
use scroll_alloy_provider::ScrollAuthApiEngineClient;
use scroll_db::Database;
use scroll_engine::{Engine, ForkchoiceState};
use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};
use tokio::sync::{mpsc, Mutex};

/// The default engine client used for engine calls.
pub type DefaultEngineClient = HttpClient<RpcLogger<RpcService<AuthClientService<HttpBackend>>>>;

/// Main test fixture providing a high-level interface for testing rollup nodes.
#[derive(Debug)]
pub struct TestFixture<EC = DefaultEngineClient> {
    /// The list of nodes in the test setup.
    pub nodes: Vec<NodeHandle<EC>>,
    /// Shared wallet for generating transactions.
    pub wallet: Arc<Mutex<Wallet>>,
    /// Chain spec used by the nodes.
    pub chain_spec: Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>,
    /// Test database (if configured).
    pub database: Option<Arc<Database>>,
    /// The task manager. Held in order to avoid dropping the node.
    _tasks: TaskManager,
}

/// The network handle to the Scroll network.
pub type ScrollNetworkHandle =
    NetworkHandle<BasicNetworkPrimitives<ScrollPrimitives, ScrollPooledTransaction>>;

/// The blockchain test provider.
pub type TestBlockChainProvider =
    BlockchainProvider<NodeTypesWithDBAdapter<ScrollRollupNode, TmpDB>>;

/// Handle to a single test node with its components.
pub struct NodeHandle<EC> {
    /// The underlying node context.
    pub node: NodeHelperType<ScrollRollupNode, TestBlockChainProvider>,
    /// Engine instance for this node.
    pub engine: Engine<ScrollAuthApiEngineClient<EC>>,
    /// L1 watcher notification channel.
    pub l1_watcher_tx: Option<mpsc::Sender<Arc<L1Notification>>>,
    /// Chain orchestrator listener.
    pub chain_orchestrator_rx: EventStream<ChainOrchestratorEvent>,
    /// Chain orchestrator handle.
    pub rollup_manager_handle: ChainOrchestratorHandle<ScrollNetworkHandle>,
    /// Node index in the test setup.
    pub index: usize,
}

impl<EC: Debug> Debug for NodeHandle<EC> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeHandle")
            .field("node", &"NodeHelper")
            .field("engine", &self.engine)
            .field("l1_watcher_tx", &self.l1_watcher_tx)
            .field("rollup_manager_handle", &self.rollup_manager_handle)
            .field("index", &self.index)
            .finish()
    }
}

impl<EC> TestFixture<EC> {
    /// Create a new test fixture builder for a sequencer node.
    pub fn sequencer() -> TestFixtureBuilder {
        TestFixtureBuilder::new().with_sequencer()
    }

    /// Create a new test fixture builder for follower nodes.
    pub fn followers(count: usize) -> TestFixtureBuilder {
        TestFixtureBuilder::new().with_nodes(count)
    }

    /// Create a new test fixture builder with custom configuration.
    pub fn builder() -> TestFixtureBuilder {
        TestFixtureBuilder::new()
    }

    /// Get the sequencer node (assumes first node is sequencer).
    pub fn sequencer_node(&mut self) -> &mut NodeHandle<EC>
    where
        EC: EngineApiClient<ScrollEngineTypes> + Send + Sync + 'static,
    {
        &mut self.nodes[0]
    }

    /// Get a follower node by index.
    pub fn follower_node(&mut self, index: usize) -> &mut NodeHandle<EC> {
        &mut self.nodes[index + 1]
    }

    /// Get the wallet address.
    pub fn wallet_address(&self) -> Address {
        self.wallet.blocking_lock().inner.address()
    }

    /// Start building a block using the sequencer.
    pub fn build_block(&mut self) -> BlockBuilder<'_, EC>
    where
        EC: EngineApiClient<ScrollEngineTypes> + Send + Sync + 'static,
    {
        BlockBuilder::new(self)
    }

    /// Get L1 helper for managing L1 interactions.
    pub fn l1(&mut self) -> L1Helper<'_, EC> {
        L1Helper::new(self)
    }

    /// Get transaction helper for creating and injecting transactions.
    pub fn tx(&mut self) -> TxHelper<'_, EC> {
        TxHelper::new(self)
    }

    /// Inject a simple transfer transaction and return its hash.
    pub async fn inject_transfer(&mut self) -> eyre::Result<alloy_primitives::B256> {
        self.tx().transfer().inject().await
    }

    /// Advance time by sleeping for the specified number of seconds.
    pub async fn advance_time(&self, seconds: u64) {
        tokio::time::sleep(tokio::time::Duration::from_secs(seconds)).await;
    }

    /// Connect all nodes in a mesh network.
    pub async fn connect_all(&mut self) {
        let node_count = self.nodes.len();
        for i in 0..node_count {
            for j in (i + 1)..node_count {
                self.connect(i, j).await;
            }
        }
    }

    /// Connect two nodes by index.
    pub async fn connect(&mut self, idx1: usize, idx2: usize) {
        // Split the nodes vector to get mutable references to both nodes
        if idx1 == idx2 {
            return;
        }

        let (node1, node2) = if idx1 < idx2 {
            let (left, right) = self.nodes.split_at_mut(idx2);
            (&mut left[idx1], &mut right[0])
        } else {
            let (left, right) = self.nodes.split_at_mut(idx1);
            (&mut right[0], &mut left[idx2])
        };

        node1.node.connect(&mut node2.node).await;
    }

    /// Get the genesis hash from the chain spec.
    pub fn genesis_hash(&self) -> BlockHash {
        self.chain_spec.genesis_hash()
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> u64 {
        self.chain_spec.chain().into()
    }
}

/// Builder for creating test fixtures with a fluent API.
#[derive(Debug)]
pub struct TestFixtureBuilder {
    config_builder: TestConfigBuilder,
    num_nodes: usize,
    chain_spec: Option<Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>>,
    is_dev: bool,
    no_local_transactions_propagation: bool,
    with_database: bool,
}

impl Default for TestFixtureBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestFixtureBuilder {
    /// Create a new test fixture builder.
    pub fn new() -> Self {
        Self {
            config_builder: TestConfigBuilder::new(),
            num_nodes: 1,
            chain_spec: None,
            is_dev: false,
            no_local_transactions_propagation: false,
            with_database: false,
        }
    }

    /// Enable sequencer mode with default settings.
    pub fn with_sequencer(mut self) -> Self {
        self.config_builder.with_sequencer();
        self
    }

    /// Set the number of nodes to create.
    pub const fn with_nodes(mut self, count: usize) -> Self {
        self.num_nodes = count;
        self
    }

    /// Set a custom chain spec.
    pub fn with_chain_spec(
        mut self,
        spec: Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>,
    ) -> Self {
        self.chain_spec = Some(spec);
        self
    }

    /// Enable dev mode.
    pub const fn with_dev_mode(mut self, enabled: bool) -> Self {
        self.is_dev = enabled;
        self
    }

    /// Disable local transaction propagation.
    pub const fn no_local_tx_propagation(mut self) -> Self {
        self.no_local_transactions_propagation = true;
        self
    }

    /// Include a test database in the fixture.
    pub const fn with_test_database(mut self) -> Self {
        self.with_database = true;
        self
    }

    /// Set the block time for the sequencer.
    pub fn block_time(mut self, millis: u64) -> Self {
        self.config_builder.block_time(millis);
        self
    }

    /// Set whether to allow empty blocks.
    pub fn allow_empty_blocks(mut self, allow: bool) -> Self {
        self.config_builder.allow_empty_blocks(allow);
        self
    }

    /// Set L1 message inclusion mode with block depth.
    pub fn with_l1_message_delay(mut self, depth: u64) -> Self {
        self.config_builder.with_l1_message_delay(depth);
        self
    }

    /// Get mutable access to the config builder for advanced customization.
    pub fn config_mut(&mut self) -> &mut TestConfigBuilder {
        &mut self.config_builder
    }

    /// Build the test fixture.
    pub async fn build(self) -> eyre::Result<TestFixture<impl EngineApiClient<ScrollEngineTypes>>> {
        let config = self.config_builder.build();
        let chain_spec = self.chain_spec.unwrap_or_else(|| SCROLL_DEV.clone());

        let (nodes, _tasks, wallet) = setup_engine(
            config,
            self.num_nodes,
            chain_spec.clone(),
            self.is_dev,
            self.no_local_transactions_propagation,
        )
        .await?;

        #[allow(clippy::if_then_some_else_none)]
        let database = if self.with_database {
            Some(Arc::new(scroll_db::test_utils::setup_test_db().await))
        } else {
            None
        };

        let mut node_handles = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.into_iter().enumerate() {
            let genesis_hash = node.inner.chain_spec().genesis_hash();

            // Create engine for the node
            let auth_client = node.inner.engine_http_client();
            let engine_client = ScrollAuthApiEngineClient::new(auth_client);
            let fcs = ForkchoiceState::new(
                BlockInfo { hash: genesis_hash, number: 0 },
                Default::default(),
                Default::default(),
            );
            let engine = Engine::new(Arc::new(engine_client), fcs);

            // Get handles if available
            let l1_watcher_tx = node.inner.add_ons_handle.l1_watcher_tx.clone();
            let rollup_manager_handle = node.inner.add_ons_handle.rollup_manager_handle.clone();
            let chain_orchestrator_rx =
                node.inner.add_ons_handle.rollup_manager_handle.get_event_listener().await?;

            node_handles.push(NodeHandle {
                node,
                engine,
                chain_orchestrator_rx,
                l1_watcher_tx,
                rollup_manager_handle,
                index,
            });
        }

        Ok(TestFixture {
            nodes: node_handles,
            wallet: Arc::new(Mutex::new(wallet)),
            chain_spec,
            database,
            _tasks,
        })
    }
}
