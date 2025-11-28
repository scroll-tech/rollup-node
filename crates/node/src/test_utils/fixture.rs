//! Core test fixture for setting up and managing test nodes.

use super::{
    block_builder::BlockBuilder, l1_helpers::L1Helper, setup_engine, tx_helpers::TxHelper,
};
use crate::{
    BlobProviderArgs, ChainOrchestratorArgs, ConsensusAlgorithm, ConsensusArgs, EngineDriverArgs,
    L1ProviderArgs, RollupNodeDatabaseArgs, RollupNodeGasPriceOracleArgs, RollupNodeNetworkArgs,
    RpcArgs, ScrollRollupNode, ScrollRollupNodeConfig, SequencerArgs, SignerArgs, TestArgs,
};

use alloy_eips::BlockNumberOrTag;
use alloy_primitives::Address;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::Block;
use alloy_signer_local::PrivateKeySigner;
use reth_chainspec::EthChainSpec;
use reth_e2e_test_utils::{wallet::Wallet, NodeHelperType, TmpDB};
use reth_eth_wire_types::BasicNetworkPrimitives;
use reth_network::NetworkHandle;
use reth_node_builder::NodeTypes;
use reth_node_types::NodeTypesWithDBAdapter;
use reth_provider::providers::BlockchainProvider;
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_primitives::ScrollPrimitives;
use reth_tasks::TaskManager;
use reth_tokio_util::EventStream;
use rollup_node_chain_orchestrator::{ChainOrchestratorEvent, ChainOrchestratorHandle};
use rollup_node_primitives::BlockInfo;
use rollup_node_sequencer::L1MessageInclusionMode;
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::ScrollPooledTransaction;
use scroll_alloy_provider::{ScrollAuthApiEngineClient, ScrollEngineApi};
use scroll_alloy_rpc_types::Transaction;
use scroll_engine::{Engine, ForkchoiceState};
use std::{
    fmt::{Debug, Formatter},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{mpsc, Mutex};

/// Main test fixture providing a high-level interface for testing rollup nodes.
pub struct TestFixture {
    /// The list of nodes in the test setup.
    pub nodes: Vec<NodeHandle>,
    /// Shared wallet for generating transactions.
    pub wallet: Arc<Mutex<Wallet>>,
    /// Chain spec used by the nodes.
    pub chain_spec: Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>,
    /// Optional Anvil instance for L1 simulation.
    pub anvil: Option<anvil::NodeHandle>,
    /// The task manager. Held in order to avoid dropping the node.
    _tasks: TaskManager,
}

impl Debug for TestFixture {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestFixture")
            .field("nodes", &self.nodes)
            .field("wallet", &"<Mutex<Wallet>>")
            .field("chain_spec", &self.chain_spec)
            .field("anvil", &self.anvil.is_some())
            .field("_tasks", &"<TaskManager>")
            .finish()
    }
}

/// The network handle to the Scroll network.
pub type ScrollNetworkHandle =
    NetworkHandle<BasicNetworkPrimitives<ScrollPrimitives, ScrollPooledTransaction>>;

/// The blockchain test provider.
pub type TestBlockChainProvider =
    BlockchainProvider<NodeTypesWithDBAdapter<ScrollRollupNode, TmpDB>>;

/// The node type (sequencer or follower).
#[derive(Debug)]
pub enum NodeType {
    /// A sequencer node.
    Sequencer,
    /// A follower node.
    Follower,
}

/// Handle to a single test node with its components.
pub struct NodeHandle {
    /// The underlying node context.
    pub node: NodeHelperType<ScrollRollupNode, TestBlockChainProvider>,
    /// Engine instance for this node.
    pub engine: Engine<Arc<dyn ScrollEngineApi + Send + Sync + 'static>>,
    /// L1 watcher notification channel.
    pub l1_watcher_tx: Option<mpsc::Sender<Arc<L1Notification>>>,
    /// Chain orchestrator listener.
    pub chain_orchestrator_rx: EventStream<ChainOrchestratorEvent>,
    /// Chain orchestrator handle.
    pub rollup_manager_handle: ChainOrchestratorHandle<ScrollNetworkHandle>,
    /// The type of the node.
    pub typ: NodeType,
}

impl NodeHandle {
    /// Returns true if this is a handle to the sequencer.
    pub const fn is_sequencer(&self) -> bool {
        matches!(self.typ, NodeType::Sequencer)
    }

    /// Returns true if this is a handle to a follower.
    pub const fn is_follower(&self) -> bool {
        matches!(self.typ, NodeType::Follower)
    }
}

impl Debug for NodeHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeHandle")
            .field("node", &"NodeHelper")
            .field("engine", &"Box<dyn ScrollEngineApi>")
            .field("l1_watcher_tx", &self.l1_watcher_tx)
            .field("rollup_manager_handle", &self.rollup_manager_handle)
            .finish()
    }
}

impl TestFixture {
    /// Create a new test fixture builder with custom configuration.
    pub fn builder() -> TestFixtureBuilder {
        TestFixtureBuilder::new()
    }

    /// Get the sequencer node (assumes first node is sequencer).
    pub fn sequencer(&mut self) -> &mut NodeHandle {
        let handle = &mut self.nodes[0];
        assert!(handle.is_sequencer(), "expected sequencer, got follower");
        handle
    }

    /// Get a follower node by index.
    pub fn follower(&mut self, index: usize) -> &mut NodeHandle {
        if index == 0 && self.nodes[0].is_sequencer() {
            return &mut self.nodes[index + 1];
        }
        &mut self.nodes[index]
    }

    /// Get the wallet.
    pub fn wallet(&self) -> Arc<Mutex<Wallet>> {
        self.wallet.clone()
    }

    /// Start building a block using the sequencer.
    pub const fn build_block(&mut self) -> BlockBuilder<'_> {
        BlockBuilder::new(self)
    }

    /// Get L1 helper for managing L1 interactions.
    pub const fn l1(&mut self) -> L1Helper<'_> {
        L1Helper::new(self)
    }

    /// Get transaction helper for creating and injecting transactions.
    pub const fn tx(&mut self) -> TxHelper<'_> {
        TxHelper::new(self)
    }

    /// Inject a simple transfer transaction and return its hash.
    pub async fn inject_transfer(&mut self) -> eyre::Result<alloy_primitives::B256> {
        self.tx().transfer().inject().await
    }

    /// Inject a raw transaction into a specific node's pool.
    pub async fn inject_tx_on(
        &mut self,
        node_index: usize,
        tx: impl Into<alloy_primitives::Bytes>,
    ) -> eyre::Result<()> {
        self.nodes[node_index].node.rpc.inject_tx(tx.into()).await?;
        Ok(())
    }

    /// Get the current (latest) block from a specific node.
    pub async fn get_block(&self, node_index: usize) -> eyre::Result<Block<Transaction>> {
        use reth_rpc_api::EthApiServer;

        self.nodes[node_index]
            .node
            .rpc
            .inner
            .eth_api()
            .block_by_number(BlockNumberOrTag::Latest, false)
            .await?
            .ok_or_else(|| eyre::eyre!("Latest block not found"))
    }

    /// Get the current (latest) block from the sequencer node.
    pub async fn get_sequencer_block(&self) -> eyre::Result<Block<Transaction>> {
        self.get_block(0).await
    }

    /// Get the status (including forkchoice state) from a specific node.
    pub async fn get_status(
        &self,
        node_index: usize,
    ) -> eyre::Result<rollup_node_chain_orchestrator::ChainOrchestratorStatus> {
        self.nodes[node_index]
            .rollup_manager_handle
            .status()
            .await
            .map_err(|e| eyre::eyre!("Failed to get status: {}", e))
    }

    /// Get the status (including forkchoice state) from the sequencer node.
    pub async fn get_sequencer_status(
        &self,
    ) -> eyre::Result<rollup_node_chain_orchestrator::ChainOrchestratorStatus> {
        self.get_status(0).await
    }

    /// Get the Anvil HTTP endpoint if Anvil was started.
    pub fn anvil_endpoint(&self) -> Option<String> {
        self.anvil.as_ref().map(|a| a.http_endpoint())
    }

    /// Generate Anvil blocks by calling `anvil_mine` RPC method.
    pub async fn anvil_mine_blocks(&self, num_blocks: u64) -> eyre::Result<()> {
        // Ensure Anvil is running
        let anvil_endpoint =
            self.anvil_endpoint().ok_or_else(|| eyre::eyre!("Anvil is not running"))?;

        // Create RPC client
        let client = alloy_rpc_client::RpcClient::new_http(anvil_endpoint.parse()?);

        // Mine blocks using anvil_mine RPC method
        // Parameters: (num_blocks, interval_in_seconds)
        let _: () = client.request("anvil_mine", (num_blocks, 0)).await?;

        Ok(())
    }

    /// Send a raw transaction to Anvil.
    pub async fn anvil_send_raw_transaction(
        &self,
        raw_tx: impl Into<alloy_primitives::Bytes>,
    ) -> eyre::Result<alloy_primitives::B256> {
        // Ensure Anvil is running
        let anvil_endpoint =
            self.anvil_endpoint().ok_or_else(|| eyre::eyre!("Anvil is not running"))?;

        // Create provider
        let provider = ProviderBuilder::new().connect_http(anvil_endpoint.parse()?);

        // Send raw transaction
        let raw_tx_bytes = raw_tx.into();
        let pending_tx = provider.send_raw_transaction(&raw_tx_bytes).await?;

        let tx_hash = *pending_tx.tx_hash();
        tracing::info!("Sent raw transaction to Anvil: {:?}", tx_hash);

        Ok(tx_hash)
    }

    /// Reorg Anvil by a specific depth (number of blocks to rewind).
    pub async fn anvil_reorg(&self, depth: u64) -> eyre::Result<()> {
        // Ensure Anvil is running
        let anvil_endpoint =
            self.anvil_endpoint().ok_or_else(|| eyre::eyre!("Anvil is not running"))?;

        // Create RPC client
        let client = alloy_rpc_client::RpcClient::new_http(anvil_endpoint.parse()?);

        // Call anvil_reorg
        // Parameters: (depth, transactions)
        // - depth: number of blocks to rewind from current head
        // - transactions: empty array means reorg without adding new transactions
        let _: () = client.request("anvil_reorg", (depth, Vec::<String>::new())).await?;

        tracing::info!("Reorged Anvil by {} blocks", depth);

        Ok(())
    }
}

/// Configuration for Anvil L1 simulation.
#[derive(Debug, Default, Clone)]
pub struct AnvilConfig {
    /// Whether to enable Anvil.
    pub enabled: bool,
    /// Optional state file to load into Anvil.
    pub state_path: Option<PathBuf>,
    /// Optional chain ID for Anvil.
    pub chain_id: Option<u64>,
    /// Optional block time for Anvil (in seconds).
    pub block_time: Option<u64>,
}

/// Builder for creating test fixtures with a fluent API.
#[derive(Debug)]
pub struct TestFixtureBuilder {
    config: ScrollRollupNodeConfig,
    num_nodes: usize,
    chain_spec: Option<Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>>,
    is_dev: bool,
    no_local_transactions_propagation: bool,
    anvil_config: AnvilConfig,
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
            config: Self::default_config(),
            num_nodes: 0,
            chain_spec: None,
            is_dev: false,
            no_local_transactions_propagation: false,
            anvil_config: AnvilConfig::default(),
        }
    }

    /// Returns the default rollup node config.
    fn default_config() -> ScrollRollupNodeConfig {
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
            signer_args: SignerArgs::default(),
            gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
            consensus_args: ConsensusArgs::noop(),
            database: None,
            rpc_args: RpcArgs { enabled: true },
        }
    }

    /// Adds a sequencer node to the test with default settings.
    pub fn sequencer(mut self) -> Self {
        self.config.sequencer_args.sequencer_enabled = true;
        self.config.sequencer_args.auto_start = false;
        self.config.sequencer_args.block_time = 100;
        self.config.sequencer_args.payload_building_duration = 40;
        self.config.sequencer_args.l1_message_inclusion_mode =
            L1MessageInclusionMode::BlockDepth(0);
        self.config.sequencer_args.allow_empty_blocks = true;
        self.config.database_args.rn_db_path = Some(PathBuf::from("sqlite::memory:"));

        self.num_nodes += 1;
        self
    }

    /// Adds `count`s follower nodes to the test.
    pub const fn followers(mut self, count: usize) -> Self {
        self.num_nodes += count;
        self
    }

    /// Toggle the test field.
    pub const fn with_test(mut self, test: bool) -> Self {
        self.config.test_args.test = test;
        self
    }

    /// Enable test mode to skip L1 watcher Synced notifications.
    /// This is useful for tests that don't want to wait for L1 sync completion events.
    pub const fn skip_l1_synced_notifications(mut self) -> Self {
        self.config.test_args.skip_l1_synced = true;
        self
    }

    /// Set the sequencer url for the node.
    pub fn with_sequencer_url(mut self, url: String) -> Self {
        self.config.network_args.sequencer_url = Some(url);
        self
    }

    /// Set the sequencer auto start for the node.
    pub const fn with_sequencer_auto_start(mut self, auto_start: bool) -> Self {
        self.config.sequencer_args.auto_start = auto_start;
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

    /// Set the block time for the sequencer.
    pub const fn block_time(mut self, millis: u64) -> Self {
        self.config.sequencer_args.block_time = millis;
        self
    }

    /// Set whether to allow empty blocks.
    pub const fn allow_empty_blocks(mut self, allow: bool) -> Self {
        self.config.sequencer_args.allow_empty_blocks = allow;
        self
    }

    /// Set L1 message inclusion mode with block depth.
    pub const fn with_l1_message_delay(mut self, depth: u64) -> Self {
        self.config.sequencer_args.l1_message_inclusion_mode =
            L1MessageInclusionMode::BlockDepth(depth);
        self
    }

    /// Set L1 message inclusion mode to finalized with optional block depth.
    pub const fn with_finalized_l1_messages(mut self, depth: u64) -> Self {
        self.config.sequencer_args.l1_message_inclusion_mode =
            L1MessageInclusionMode::FinalizedWithBlockDepth(depth);
        self
    }

    /// Use an in-memory `SQLite` database.
    pub fn with_memory_db(mut self) -> Self {
        self.config.database_args.rn_db_path = Some(PathBuf::from("sqlite::memory:"));
        self
    }

    /// Set a custom database path.
    pub fn with_db_path(mut self, path: PathBuf) -> Self {
        self.config.database_args.rn_db_path = Some(path);
        self
    }

    /// Use noop consensus (no validation).
    pub const fn with_noop_consensus(mut self) -> Self {
        self.config.consensus_args = ConsensusArgs::noop();
        self
    }

    /// Use `SystemContract` consensus with the given authorized signer address.
    pub const fn with_consensus_system_contract(mut self, authorized_signer: Address) -> Self {
        self.config.consensus_args.algorithm = ConsensusAlgorithm::SystemContract;
        self.config.consensus_args.authorized_signer = Some(authorized_signer);
        self
    }

    /// Set the private key signer for the node.
    pub fn with_signer(mut self, signer: PrivateKeySigner) -> Self {
        self.config.signer_args.private_key = Some(signer);
        self
    }

    /// Set the payload building duration in milliseconds.
    pub const fn payload_building_duration(mut self, millis: u64) -> Self {
        self.config.sequencer_args.payload_building_duration = millis;
        self
    }

    /// Set the fee recipient address.
    pub const fn fee_recipient(mut self, address: Address) -> Self {
        self.config.sequencer_args.fee_recipient = address;
        self
    }

    /// Enable auto-start for the sequencer.
    pub const fn auto_start(mut self, enabled: bool) -> Self {
        self.config.sequencer_args.auto_start = enabled;
        self
    }

    /// Set the maximum number of L1 messages per block.
    pub const fn max_l1_messages(mut self, max: u64) -> Self {
        self.config.sequencer_args.max_l1_messages = Some(max);
        self
    }

    /// Enable the Scroll wire protocol.
    pub const fn with_scroll_wire(mut self, enabled: bool) -> Self {
        self.config.network_args.enable_scroll_wire = enabled;
        self
    }

    /// Enable the ETH-Scroll wire bridge.
    pub const fn with_eth_scroll_bridge(mut self, enabled: bool) -> Self {
        self.config.network_args.enable_eth_scroll_wire_bridge = enabled;
        self
    }

    /// Set the optimistic sync trigger threshold.
    pub const fn optimistic_sync_trigger(mut self, blocks: u64) -> Self {
        self.config.chain_orchestrator_args.optimistic_sync_trigger = blocks;
        self
    }

    /// Get a mutable reference to the underlying config for advanced customization.
    pub const fn config_mut(&mut self) -> &mut ScrollRollupNodeConfig {
        &mut self.config
    }

    /// Enable Anvil with the default state file (`./tests/testdata/anvil_state.json`).
    pub fn with_anvil(mut self) -> Self {
        self.anvil_config.enabled = true;
        self.anvil_config.state_path = Some(PathBuf::from("./tests/testdata/anvil_state.json"));
        self
    }

    /// Enable Anvil with a custom state file.
    pub fn with_anvil_custom_state(mut self, path: impl Into<PathBuf>) -> Self {
        self.anvil_config.enabled = true;
        self.anvil_config.state_path = Some(path.into());
        self
    }

    /// Set the chain ID for Anvil.
    pub const fn with_anvil_chain_id(mut self, chain_id: u64) -> Self {
        self.anvil_config.chain_id = Some(chain_id);
        self
    }

    /// Set the block time for Anvil (in seconds).
    pub const fn with_anvil_block_time(mut self, block_time: u64) -> Self {
        self.anvil_config.block_time = Some(block_time);
        self
    }

    /// Build the test fixture.
    pub async fn build(self) -> eyre::Result<TestFixture> {
        let mut config = self.config;
        let chain_spec = self.chain_spec.unwrap_or_else(|| SCROLL_DEV.clone());

        // Start Anvil if requested
        let anvil = if self.anvil_config.enabled {
            let handle = Self::spawn_anvil(
                self.anvil_config.state_path.as_deref(),
                self.anvil_config.chain_id,
                self.anvil_config.block_time,
            )
            .await?;

            // Parse endpoint URL once and reuse
            let endpoint_url = handle
                .http_endpoint()
                .parse::<reqwest::Url>()
                .map_err(|e| eyre::eyre!("Failed to parse Anvil endpoint URL: {}", e))?;

            // Configure L1 provider and blob provider to use Anvil
            config.l1_provider_args.url = Some(endpoint_url.clone());
            config.l1_provider_args.logs_query_block_range = 500;
            config.blob_provider_args.anvil_url = Some(endpoint_url);
            config.blob_provider_args.mock = false;

            Some(handle)
        } else {
            None
        };

        let (nodes, _tasks, wallet) = setup_engine(
            config.clone(),
            self.num_nodes,
            chain_spec.clone(),
            self.is_dev,
            self.no_local_transactions_propagation,
        )
        .await?;

        let mut node_handles = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.into_iter().enumerate() {
            let genesis_hash = node.inner.chain_spec().genesis_hash();

            // Create engine for the node
            let auth_client = node.inner.engine_http_client();
            let engine_client = Arc::new(ScrollAuthApiEngineClient::new(auth_client))
                as Arc<dyn ScrollEngineApi + Send + Sync + 'static>;
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
                typ: if config.sequencer_args.sequencer_enabled && index == 0 {
                    NodeType::Sequencer
                } else {
                    NodeType::Follower
                },
            });
        }

        Ok(TestFixture {
            nodes: node_handles,
            wallet: Arc::new(Mutex::new(wallet)),
            chain_spec,
            _tasks,
            anvil,
        })
    }

    /// Spawn an Anvil instance with the given configuration.
    async fn spawn_anvil(
        state_path: Option<&std::path::Path>,
        chain_id: Option<u64>,
        block_time: Option<u64>,
    ) -> eyre::Result<anvil::NodeHandle> {
        let mut config = anvil::NodeConfig::default();

        // Configure chain ID
        if let Some(id) = chain_id {
            config.chain_id = Some(id);
        }

        config.port = 8544;

        // Configure block time
        if let Some(time) = block_time {
            config.block_time = Some(std::time::Duration::from_secs(time));
        }

        // Load state from file if provided
        if let Some(path) = state_path {
            let state = anvil::eth::backend::db::SerializableState::load(path).map_err(|e| {
                eyre::eyre!("Failed to load Anvil state from {}: {:?}", path.display(), e)
            })?;
            tracing::info!("Loaded Anvil state from: {}", path.display());
            config.init_state = Some(state);
        }

        // Spawn Anvil and return the NodeHandle
        let (_api, handle) = anvil::spawn(config).await;
        Ok(handle)
    }
}
