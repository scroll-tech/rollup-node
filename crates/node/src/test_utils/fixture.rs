//! Core test fixture for setting up and managing test nodes.

use super::{
    block_builder::BlockBuilder, l1_helpers::L1Helper, setup_engine, tx_helpers::TxHelper,
};
use crate::{
    BlobProviderArgs, ChainOrchestratorArgs, ConsensusAlgorithm, ConsensusArgs, EngineDriverArgs,
    L1ProviderArgs, RollupNodeDatabaseArgs, RollupNodeGasPriceOracleArgs, RollupNodeNetworkArgs,
    RpcArgs, ScrollRollupNode, ScrollRollupNodeConfig, SequencerArgs, SignerArgs,
};

use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{Address, BlockHash};
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
#[derive(Debug)]
pub struct TestFixture {
    /// The list of nodes in the test setup.
    pub nodes: Vec<NodeHandle>,
    /// Shared wallet for generating transactions.
    pub wallet: Arc<Mutex<Wallet>>,
    /// Chain spec used by the nodes.
    pub chain_spec: Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>,
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
    /// Node index in the test setup.
    pub index: usize,
}

impl Debug for NodeHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeHandle")
            .field("node", &"NodeHelper")
            .field("engine", &"Box<dyn ScrollEngineApi>")
            .field("l1_watcher_tx", &self.l1_watcher_tx)
            .field("rollup_manager_handle", &self.rollup_manager_handle)
            .field("index", &self.index)
            .finish()
    }
}

impl TestFixture {
    /// Create a new test fixture builder with custom configuration.
    pub fn builder() -> TestFixtureBuilder {
        TestFixtureBuilder::new()
    }

    /// Get the sequencer node (assumes first node is sequencer).
    pub fn sequencer_node(&mut self) -> &mut NodeHandle {
        &mut self.nodes[0]
    }

    /// Get a follower node by index.
    pub fn follower_node(&mut self, index: usize) -> &mut NodeHandle {
        &mut self.nodes[index + 1]
    }

    /// Get the wallet address.
    pub fn wallet_address(&self) -> Address {
        self.wallet.blocking_lock().inner.address()
    }

    /// Start building a block using the sequencer.
    pub fn build_block(&mut self) -> BlockBuilder<'_> {
        BlockBuilder::new(self)
    }

    /// Get L1 helper for managing L1 interactions.
    pub fn l1(&mut self) -> L1Helper<'_> {
        L1Helper::new(self)
    }

    /// Get transaction helper for creating and injecting transactions.
    pub fn tx(&mut self) -> TxHelper<'_> {
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

    /// Get the RPC URL for a specific node.
    pub fn rpc_url(&self, node_index: usize) -> String {
        format!("http://localhost:{}", self.nodes[node_index].node.rpc_url().port().unwrap())
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

    /// Get a block by number from a specific node.
    pub async fn get_block_by_number(
        &self,
        node_index: usize,
        block_number: impl Into<BlockNumberOrTag>,
    ) -> eyre::Result<Option<Block<Transaction>>> {
        use reth_rpc_api::EthApiServer;

        self.nodes[node_index]
            .node
            .rpc
            .inner
            .eth_api()
            .block_by_number(block_number.into(), false)
            .await
            .map_err(Into::into)
    }
}

/// Builder for creating test fixtures with a fluent API.
#[derive(Debug)]
pub struct TestFixtureBuilder {
    config: ScrollRollupNodeConfig,
    num_nodes: usize,
    chain_spec: Option<Arc<<ScrollRollupNode as NodeTypes>::ChainSpec>>,
    is_dev: bool,
    no_local_transactions_propagation: bool,
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
            num_nodes: 1,
            chain_spec: None,
            is_dev: false,
            no_local_transactions_propagation: false,
        }
    }

    /// Returns the default rollup node config.
    fn default_config() -> ScrollRollupNodeConfig {
        ScrollRollupNodeConfig {
            test: true,
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
        self
    }

    /// Adds `count`s follower nodes to the test.
    pub fn followers(mut self, count: usize) -> TestFixtureBuilder {
        self.num_nodes = count;
        self
    }

    /// Toggle the test field.
    pub fn with_test(mut self, test: bool) -> Self {
        self.config.test = test;
        self
    }

    /// Set the sequencer url for the node.
    pub fn with_sequencer_url(mut self, url: String) -> Self {
        self.config.network_args.sequencer_url = Some(url);
        self
    }

    /// Set the sequencer auto start for the node.
    pub fn with_sequencer_auto_start(mut self, auto_start: bool) -> Self {
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
    pub fn block_time(mut self, millis: u64) -> Self {
        self.config.sequencer_args.block_time = millis;
        self
    }

    /// Set whether to allow empty blocks.
    pub fn allow_empty_blocks(mut self, allow: bool) -> Self {
        self.config.sequencer_args.allow_empty_blocks = allow;
        self
    }

    /// Set L1 message inclusion mode with block depth.
    pub fn with_l1_message_delay(mut self, depth: u64) -> Self {
        self.config.sequencer_args.l1_message_inclusion_mode =
            L1MessageInclusionMode::BlockDepth(depth);
        self
    }

    /// Set L1 message inclusion mode to finalized with optional block depth.
    pub fn with_finalized_l1_messages(mut self, depth: u64) -> Self {
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
    pub fn with_noop_consensus(mut self) -> Self {
        self.config.consensus_args = ConsensusArgs::noop();
        self
    }

    /// Use SystemContract consensus with the given authorized signer address.
    pub fn with_consensus_system_contract(mut self, authorized_signer: Address) -> Self {
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
    pub fn payload_building_duration(mut self, millis: u64) -> Self {
        self.config.sequencer_args.payload_building_duration = millis;
        self
    }

    /// Set the fee recipient address.
    pub fn fee_recipient(mut self, address: Address) -> Self {
        self.config.sequencer_args.fee_recipient = address;
        self
    }

    /// Enable auto-start for the sequencer.
    pub fn auto_start(mut self, enabled: bool) -> Self {
        self.config.sequencer_args.auto_start = enabled;
        self
    }

    /// Set the maximum number of L1 messages per block.
    pub fn max_l1_messages(mut self, max: u64) -> Self {
        self.config.sequencer_args.max_l1_messages = Some(max);
        self
    }

    /// Enable the Scroll wire protocol.
    pub fn with_scroll_wire(mut self, enabled: bool) -> Self {
        self.config.network_args.enable_scroll_wire = enabled;
        self
    }

    /// Enable the ETH-Scroll wire bridge.
    pub fn with_eth_scroll_bridge(mut self, enabled: bool) -> Self {
        self.config.network_args.enable_eth_scroll_wire_bridge = enabled;
        self
    }

    /// Set the optimistic sync trigger threshold.
    pub fn optimistic_sync_trigger(mut self, blocks: u64) -> Self {
        self.config.chain_orchestrator_args.optimistic_sync_trigger = blocks;
        self
    }

    /// Set the chain buffer size.
    pub fn chain_buffer_size(mut self, size: usize) -> Self {
        self.config.chain_orchestrator_args.chain_buffer_size = size;
        self
    }

    /// Disable the test mode (enables real signing).
    pub fn production_mode(mut self) -> Self {
        self.config.test = false;
        self
    }

    /// Get a mutable reference to the underlying config for advanced customization.
    pub fn config_mut(&mut self) -> &mut ScrollRollupNodeConfig {
        &mut self.config
    }

    /// Build the test fixture.
    pub async fn build(self) -> eyre::Result<TestFixture> {
        let config = self.config;
        let chain_spec = self.chain_spec.unwrap_or_else(|| SCROLL_DEV.clone());

        let (nodes, _tasks, wallet) = setup_engine(
            config,
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
                index,
            });
        }

        Ok(TestFixture {
            nodes: node_handles,
            wallet: Arc::new(Mutex::new(wallet)),
            chain_spec,
            _tasks,
        })
    }
}
