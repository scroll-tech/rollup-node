use crate::{
    add_ons::IsDevChain,
    constants::{self},
    context::RollupNodeContext,
};
use scroll_migration::MigratorTrait;
use std::{fs, path::PathBuf, sync::Arc};

use alloy_chains::NamedChain;
use alloy_primitives::{hex, Address, U128};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_client::RpcClient;
use alloy_signer::Signer;
use alloy_signer_aws::AwsSigner;
use alloy_signer_local::PrivateKeySigner;
use alloy_transport::layers::RetryBackoffLayer;
use aws_sdk_kms::config::BehaviorVersion;
use clap::ArgAction;
use reth_chainspec::EthChainSpec;
use reth_network::NetworkProtocols;
use reth_network_api::FullNetwork;
use reth_network_p2p::FullBlockClient;
use reth_node_builder::{rpc::RethRpcServerHandles, NodeConfig as RethNodeConfig};
use reth_node_core::primitives::BlockHeader;
use reth_scroll_chainspec::{
    ChainConfig, ScrollChainConfig, ScrollChainSpec, SCROLL_FEE_VAULT_ADDRESS,
};
use reth_scroll_consensus::ScrollBeaconConsensus;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_chain_orchestrator::{
    ChainOrchestrator, ChainOrchestratorConfig, ChainOrchestratorHandle, Consensus, NoopConsensus,
    SystemContractConsensus,
};
use rollup_node_primitives::{BlockInfo, NodeConfig};
use rollup_node_providers::{
    BlobProvidersBuilder, FullL1Provider, L1MessageProvider, SystemContractProvider,
};
use rollup_node_sequencer::{
    L1MessageInclusionMode, PayloadBuildingConfig, Sequencer, SequencerConfig,
};
use rollup_node_watcher::{L1Notification, L1Watcher};
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_network::Scroll;
use scroll_alloy_provider::{ScrollAuthApiEngineClient, ScrollEngineApi};
use scroll_db::{
    Database, DatabaseConnectionProvider, DatabaseError, DatabaseReadOperations,
    DatabaseWriteOperations,
};
use scroll_derivation_pipeline::DerivationPipeline;
use scroll_engine::{Engine, ForkchoiceState};
use scroll_migration::traits::ScrollMigrator;
use scroll_network::ScrollNetworkManager;
use scroll_wire::ScrollWireEvent;
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

/// A struct that represents the arguments for the rollup node.
#[derive(Debug, Clone, clap::Args)]
pub struct ScrollRollupNodeConfig {
    /// Whether the rollup node should be run in test mode.
    #[arg(long)]
    pub test: bool,
    /// Consensus args
    #[command(flatten)]
    pub consensus_args: ConsensusArgs,
    /// Database args
    #[command(flatten)]
    pub database_args: RollupNodeDatabaseArgs,
    /// Chain orchestrator args.
    #[command(flatten)]
    pub chain_orchestrator_args: ChainOrchestratorArgs,
    /// Engine driver args.
    #[command(flatten)]
    pub engine_driver_args: EngineDriverArgs,
    /// The blob provider arguments.
    #[command(flatten)]
    pub blob_provider_args: BlobProviderArgs,
    /// The L1 provider arguments
    #[command(flatten)]
    pub l1_provider_args: L1ProviderArgs,
    /// The sequencer arguments
    #[command(flatten)]
    pub sequencer_args: SequencerArgs,
    /// The network arguments
    #[command(flatten)]
    pub network_args: RollupNodeNetworkArgs,
    /// The rpc arguments
    #[command(flatten)]
    pub rpc_args: RpcArgs,
    /// The signer arguments
    #[command(flatten)]
    pub signer_args: SignerArgs,
    /// The gas price oracle args
    #[command(flatten)]
    pub gas_price_oracle_args: RollupNodeGasPriceOracleArgs,
    /// The database connection (not parsed via CLI but hydrated after validation).
    #[arg(skip)]
    pub database: Option<Arc<Database>>,
}

impl ScrollRollupNodeConfig {
    /// Validate that either signer key file or AWS KMS key ID is provided when sequencer is enabled
    pub fn validate(&self) -> Result<(), String> {
        if self.sequencer_args.sequencer_enabled &
            !matches!(self.consensus_args.algorithm, ConsensusAlgorithm::Noop)
        {
            if self.signer_args.key_file.is_none() &&
                self.signer_args.aws_kms_key_id.is_none() &&
                self.signer_args.private_key.is_none()
            {
                return Err("Either signer key file, AWS KMS key ID or private key is required when sequencer is enabled".to_string());
            }

            if (self.signer_args.key_file.is_some() as u8 +
                self.signer_args.aws_kms_key_id.is_some() as u8 +
                self.signer_args.private_key.is_some() as u8) >
                1
            {
                return Err("Cannot specify more than one signer key source".to_string());
            }
        }

        if self.consensus_args.algorithm == ConsensusAlgorithm::SystemContract &&
            self.consensus_args.authorized_signer.is_none() &&
            self.l1_provider_args.url.is_none()
        {
            return Err("System contract consensus requires either an authorized signer or a L1 provider URL".to_string());
        }

        Ok(())
    }

    /// Hydrate the config by initializing the database connection.
    pub async fn hydrate(
        &mut self,
        node_config: RethNodeConfig<ScrollChainSpec>,
    ) -> eyre::Result<()> {
        // Instantiate the database
        let db_path = node_config.datadir().db();
        let database_path = if let Some(database_path) = &self.database_args.rn_db_path {
            database_path.to_string_lossy().to_string()
        } else {
            // append the path using strings as using `join(...)` overwrites "sqlite://"
            // if the path is absolute.
            let path = db_path.join("scroll.db?mode=rwc");
            "sqlite://".to_string() + &*path.to_string_lossy()
        };
        let db = Database::new(&database_path).await?;
        self.database = Some(Arc::new(db));
        Ok(())
    }
}

impl ScrollRollupNodeConfig {
    /// Consumes the [`ScrollRollupNodeConfig`] and builds a [`ChainOrchestrator`].
    pub async fn build<N, CS>(
        self,
        ctx: RollupNodeContext<N, CS>,
        events: UnboundedReceiver<ScrollWireEvent>,
        rpc_server_handles: RethRpcServerHandles,
    ) -> eyre::Result<(
        ChainOrchestrator<
            N,
            impl ScrollHardforks + EthChainSpec<Header: BlockHeader> + IsDevChain + Clone + 'static,
            impl L1MessageProvider + Clone,
            impl Provider<Scroll> + Clone,
            impl ScrollEngineApi,
        >,
        ChainOrchestratorHandle<N>,
        Option<Sender<Arc<L1Notification>>>,
    )>
    where
        N: FullNetwork<Primitives = ScrollNetworkPrimitives> + NetworkProtocols,
        CS: EthChainSpec<Header: BlockHeader>
            + ChainConfig<Config = ScrollChainConfig>
            + ScrollHardforks
            + IsDevChain
            + 'static,
    {
        tracing::info!(target: "rollup_node::args",
            "Building rollup node with config:\n{:#?}",
            self
        );
        // Get the chain spec.
        let chain_spec = ctx.chain_spec;

        // Build NodeConfig directly from the chainspec.
        let node_config = Arc::new(NodeConfig::from_chainspec(&chain_spec)?);

        // Create the engine api client.
        let engine_api = ScrollAuthApiEngineClient::new(rpc_server_handles.auth.http_client());

        // Get a provider
        let l1_provider = self.l1_provider_args.url.clone().map(|url| {
            let L1ProviderArgs { max_retries, initial_backoff, compute_units_per_second, .. } =
                self.l1_provider_args;
            let client = RpcClient::builder()
                .layer(RetryBackoffLayer::new(
                    max_retries,
                    initial_backoff,
                    compute_units_per_second,
                ))
                .http(url);
            ProviderBuilder::new().connect_client(client)
        });

        // Init a retry provider to the execution layer.
        let retry_layer = RetryBackoffLayer::new(
            constants::L2_PROVIDER_MAX_RETRIES,
            constants::L2_PROVIDER_INITIAL_BACKOFF,
            constants::PROVIDER_COMPUTE_UNITS_PER_SECOND,
        );
        let client = RpcClient::builder().layer(retry_layer).http(
            rpc_server_handles
                .rpc
                .http_url()
                .expect("failed to get l2 rpc url")
                .parse()
                .expect("invalid l2 rpc url"),
        );
        let l2_provider = ProviderBuilder::<_, _, Scroll>::default().connect_client(client);
        let l2_provider = Arc::new(l2_provider);

        // Fetch the database from the hydrated config.
        let db = self.database.clone().expect("should hydrate config before build");

        // Run the database migrations
        if let Some(named) = chain_spec.chain().named() {
            let db_inner = Arc::clone(&db);
            named
                .migrate(db_inner.get_connection(), self.test)
                .await
                .expect("failed to perform migration");
        } else {
            // We can re use the dev migration for custom chains as data source and data hash are
            // None for both. We overwrite the default genesis hash from ScrollDevMigrationInfo to
            // match the custom chain.
            // This is a workaround due to the fact that sea orm migrations are static.
            // See https://github.com/scroll-tech/rollup-node/issues/297 for more details.
            let db_inner = Arc::clone(&db);
            scroll_migration::Migrator::<scroll_migration::ScrollDevMigrationInfo>::up(
                db_inner.get_connection(),
                None,
            )
            .await
            .expect("failed to perform migration (custom chain)");

            // insert the custom chain genesis hash into the database
            let genesis_hash = chain_spec.genesis_hash();
            db.insert_genesis_block(genesis_hash)
                .await
                .expect("failed to insert genesis block (custom chain)");

            tracing::info!(target: "scroll::node::args", ?genesis_hash, "Overwriting genesis hash for custom chain");
        }

        let chain_spec_fcs = || {
            ForkchoiceState::head_from_chain_spec(chain_spec.clone())
                .expect("failed to derive forkchoice state from chain spec")
        };
        let mut fcs =
            ForkchoiceState::from_provider(&l2_provider).await.unwrap_or_else(chain_spec_fcs);

        let genesis_hash = chain_spec.genesis_hash();
        let (l1_start_block_number, mut l2_head_block_number) = db
            .tx_mut(move |tx| async move {
                // On startup we replay the latest batch of blocks from the database as such we set
                // the safe block hash to the latest block hash associated with the
                // previous consolidated batch in the database.
                let (_startup_safe_block, l1_start_block_number) =
                    tx.prepare_on_startup(genesis_hash).await?;

                let l2_head_block_number = tx.get_l2_head_block_number().await?;

                Ok::<_, DatabaseError>((l1_start_block_number, l2_head_block_number))
            })
            .await?;

        // Loop to find the latest block that we have in the EN and purge L1 message mappings to
        // account for the startup block
        //
        // This is necessary as there is an edge case in which the EN may not have persisted the
        // latest block.
        let finalized_block_number = fcs.finalized_block_info().number;
        while l2_head_block_number > finalized_block_number {
            tracing::info!(target: "scroll::node::args", ?l2_head_block_number, "Checking for L2 head block in EN");

            // Check if the block exists in the EN and update the forkchoice state and L2 head block
            // number
            if let Some(block) = l2_provider
                .get_block(l2_head_block_number.into())
                .full()
                .await?
                .map(|b| b.into_consensus().map_transactions(|tx| tx.inner.into_inner()))
            {
                tracing::info!(target: "scroll::node::args", ?l2_head_block_number, "Found L2 head block in EN");
                let block_info: BlockInfo = (&block).into();
                fcs.update(Some(block_info), None, None)?;
                db.tx_mut(move |tx| async move {
                    tx.set_l2_head_block_number(l2_head_block_number).await?;
                    tx.purge_l1_message_to_l2_block_mappings(Some(l2_head_block_number + 1)).await
                })
                .await?;
                break;
            }

            // Decrement the L2 head block number and try again
            tracing::info!(target: "scroll::node::args", ?l2_head_block_number, "L2 head block not found in EN, decrementing");
            l2_head_block_number -= 1;
        }

        let chain_spec = Arc::new(chain_spec.clone());

        // Instantiate the network manager
        let eth_wire_listener = self
            .network_args
            .enable_eth_scroll_wire_bridge
            .then_some(ctx.network.eth_wire_block_listener().await?);

        // TODO: remove this once we deprecate l2geth.
        let authorized_signer = self.network_args.effective_signer(chain_spec.chain().named());

        let (scroll_network_manager, scroll_network_handle) = ScrollNetworkManager::from_parts(
            chain_spec.clone(),
            ctx.network.clone(),
            events,
            eth_wire_listener,
            td_constant(chain_spec.chain().named()),
            authorized_signer,
        );
        tokio::spawn(scroll_network_manager);

        tracing::info!(target: "scroll::node::args", fcs = ?fcs, payload_building_duration = ?self.sequencer_args.payload_building_duration, "Starting engine driver");
        let engine = Engine::new(Arc::new(engine_api), fcs);

        // Create the consensus.
        let authorized_signer = if let Some(provider) = l1_provider.as_ref() {
            Some(
                provider
                    .authorized_signer(node_config.address_book.system_contract_address)
                    .await?,
            )
        } else {
            None
        };
        let consensus = self.consensus_args.consensus(authorized_signer)?;

        let (l1_notification_tx, l1_notification_rx): (Option<Sender<Arc<L1Notification>>>, _) =
            if let Some(provider) = l1_provider.filter(|_| !self.test) {
                tracing::info!(target: "scroll::node::args", ?l1_start_block_number, "Starting L1 watcher");
                (
                    None,
                    Some(
                        L1Watcher::spawn(
                            provider,
                            l1_start_block_number,
                            node_config,
                            self.l1_provider_args.logs_query_block_range,
                        )
                        .await,
                    ),
                )
            } else {
                // Create a channel for L1 notifications that we can use to inject L1 messages for
                // testing
                #[cfg(feature = "test-utils")]
                {
                    let (tx, rx) = tokio::sync::mpsc::channel(1000);
                    (Some(tx), Some(rx))
                }

                #[cfg(not(feature = "test-utils"))]
                {
                    (None, None)
                }
            };

        // Construct the l1 provider.
        let l1_messages_provider = db.clone();
        let blob_providers_builder = BlobProvidersBuilder {
            beacon: self.blob_provider_args.beacon_node_urls,
            s3: self.blob_provider_args.s3_url,
            anvil: self.blob_provider_args.anvil_url,
            mock: self.blob_provider_args.mock,
        };
        let blob_provider =
            blob_providers_builder.build().await.expect("failed to construct L1 blob provider");
        let l1_provider = FullL1Provider::new(blob_provider, l1_messages_provider.clone()).await;

        // Construct the Sequencer.
        let chain_config = chain_spec.chain_config();
        let sequencer = self.sequencer_args.sequencer_enabled.then(|| {
            let args = &self.sequencer_args;
            let config = SequencerConfig {
                chain_spec: chain_spec.clone(),
                fee_recipient: args.fee_recipient,
                payload_building_config: PayloadBuildingConfig {
                    block_gas_limit: ctx.block_gas_limit,
                    max_l1_messages_per_block: self
                        .sequencer_args
                        .max_l1_messages
                        .unwrap_or(chain_config.l1_config.num_l1_messages_per_block),
                    l1_message_inclusion_mode: args.l1_message_inclusion_mode,
                },
                auto_start: args.auto_start,
                block_time: args.block_time,
                allow_empty_blocks: args.allow_empty_blocks,
                payload_building_duration: args.payload_building_duration,
            };
            Sequencer::new(Arc::new(l1_messages_provider), config)
        });

        // Instantiate the signer
        let chain_id = chain_spec.chain().id();
        let signer = if let Some(configured_signer) = self.signer_args.signer(chain_id).await? {
            // Use the signer configured by SignerArgs
            Some(rollup_node_signer::Signer::spawn(configured_signer))
        } else if self.test {
            // Use a random private key signer for testing
            Some(rollup_node_signer::Signer::spawn(PrivateKeySigner::random()))
        } else {
            None
        };

        // Instantiate the chain orchestrator
        let block_client = FullBlockClient::new(
            scroll_network_handle
                .inner()
                .fetch_client()
                .await
                .expect("failed to fetch block client"),
            Arc::new(ScrollBeaconConsensus::new(chain_spec.clone())),
        );
        let l1_v2_message_queue_start_index =
            l1_v2_message_queue_start_index(chain_spec.chain().named());
        let config: ChainOrchestratorConfig<Arc<CS>> = ChainOrchestratorConfig::new(
            chain_spec,
            self.chain_orchestrator_args.optimistic_sync_trigger,
            l1_v2_message_queue_start_index,
        );

        // Instantiate the derivation pipeline
        let derivation_pipeline = DerivationPipeline::new(
            l1_provider.clone(),
            db.clone(),
            l1_v2_message_queue_start_index,
        )
        .await;

        let (chain_orchestrator, handle) = ChainOrchestrator::new(
            db,
            config,
            Arc::new(block_client),
            l2_provider,
            l1_notification_rx.expect("L1 notification receiver should be set"),
            scroll_network_handle.into_scroll_network().await,
            consensus,
            engine,
            sequencer,
            signer,
            derivation_pipeline,
        )
        .await?;

        Ok((chain_orchestrator, handle, l1_notification_tx))
    }
}

/// The database arguments.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct RollupNodeDatabaseArgs {
    /// Database path
    #[arg(
        long = "rollup-node-db.path",
        value_name = "DB_PATH",
        help = "The database path for the rollup node database"
    )]
    pub rn_db_path: Option<PathBuf>,
}

/// The database arguments.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct ConsensusArgs {
    /// The type of consensus to use.
    #[arg(
        long = "consensus.algorithm",
        value_name = "CONSENSUS_ALGORITHM",
        default_value = "system-contract"
    )]
    pub algorithm: ConsensusAlgorithm,

    /// The optional authorized signer for system contract consensus.
    #[arg(long = "consensus.authorized-signer", value_name = "ADDRESS")]
    pub authorized_signer: Option<Address>,
}

impl ConsensusArgs {
    /// Create a new [`ConsensusArgs`] with the no-op consensus algorithm.
    pub const fn noop() -> Self {
        Self { algorithm: ConsensusAlgorithm::Noop, authorized_signer: None }
    }

    /// Creates a consensus instance based on the configured algorithm and authorized signer.
    ///
    /// The `authorized_signer` field of `ConsensusArgs` takes precedence over the
    /// `authorized_signer` parameter passed to this method.
    pub fn consensus(
        &self,
        authorized_signer: Option<Address>,
    ) -> eyre::Result<Box<dyn Consensus>> {
        match self.algorithm {
            ConsensusAlgorithm::Noop => Ok(Box::new(NoopConsensus::default())),
            ConsensusAlgorithm::SystemContract => {
                let authorized_signer = if let Some(address) = self.authorized_signer {
                    address
                } else if let Some(address) = authorized_signer {
                    address
                } else {
                    return Err(eyre::eyre!(
                        "System contract consensus requires either an authorized signer or a L1 provider URL"
                    ));
                };
                Ok(Box::new(SystemContractConsensus::new(authorized_signer)))
            }
        }
    }
}

/// The consensus algorithm to use.
#[derive(Debug, Default, clap::ValueEnum, Clone, PartialEq, Eq)]
pub enum ConsensusAlgorithm {
    /// System contract consensus with an optional authorized signer. If the authorized signer is
    /// not provided the system will use the L1 provider to query the authorized signer from L1.
    #[default]
    SystemContract,
    /// No-op consensus that does not validate blocks.
    Noop,
}

/// The engine driver args.
#[derive(Debug, Clone, clap::Args)]
pub struct EngineDriverArgs {
    /// Whether the engine driver should try to sync at start up.
    #[arg(long = "engine.sync-at-startup", num_args=0..=1, default_value_t = true)]
    pub sync_at_startup: bool,
}

impl Default for EngineDriverArgs {
    fn default() -> Self {
        Self { sync_at_startup: true }
    }
}

/// The chain orchestrator arguments.
#[derive(Debug, Clone, clap::Args)]
pub struct ChainOrchestratorArgs {
    /// The amount of block difference between the EN and the latest block received from P2P
    /// at which the engine driver triggers optimistic sync.
    #[arg(long = "chain.optimistic-sync-trigger", default_value_t = constants::BLOCK_GAP_TRIGGER)]
    pub optimistic_sync_trigger: u64,
    /// The size of the in-memory chain buffer used by the chain orchestrator.
    #[arg(long = "chain.chain-buffer-size", default_value_t = constants::CHAIN_BUFFER_SIZE)]
    pub chain_buffer_size: usize,
}

impl Default for ChainOrchestratorArgs {
    fn default() -> Self {
        Self {
            optimistic_sync_trigger: constants::BLOCK_GAP_TRIGGER,
            chain_buffer_size: constants::CHAIN_BUFFER_SIZE,
        }
    }
}

/// The network arguments.
#[derive(Debug, Clone, clap::Args)]
pub struct RollupNodeNetworkArgs {
    /// A bool to represent if new blocks should be bridged from the eth wire protocol to the
    /// scroll wire protocol.
    #[arg(long = "network.bridge", default_value_t = true, action = ArgAction::Set)]
    pub enable_eth_scroll_wire_bridge: bool,
    /// A bool that represents if the scroll wire protocol should be enabled.
    #[arg(long = "network.scroll-wire", default_value_t = true, action = ArgAction::Set)]
    pub enable_scroll_wire: bool,
    /// The URL for the Sequencer RPC. (can be both HTTP and WS)
    #[arg(
        long = "network.sequencer-url",
        id = "network_sequencer_url",
        value_name = "NETWORK_SEQUENCER_URL"
    )]
    pub sequencer_url: Option<String>,
    /// The valid signer address for the network.
    #[arg(long = "network.valid_signer", value_name = "VALID_SIGNER")]
    pub signer_address: Option<Address>,
}

impl Default for RollupNodeNetworkArgs {
    fn default() -> Self {
        Self {
            enable_eth_scroll_wire_bridge: true,
            enable_scroll_wire: true,
            sequencer_url: None,
            signer_address: None,
        }
    }
}

impl RollupNodeNetworkArgs {
    /// Get the default authorized signer address for the given chain.
    pub const fn default_authorized_signer(chain: Option<NamedChain>) -> Option<Address> {
        match chain {
            Some(NamedChain::Scroll) => Some(constants::SCROLL_MAINNET_SIGNER),
            Some(NamedChain::ScrollSepolia) => Some(constants::SCROLL_SEPOLIA_SIGNER),
            _ => None,
        }
    }

    /// Get the effective signer address, using the configured signer or falling back to default.
    pub fn effective_signer(&self, chain: Option<NamedChain>) -> Option<Address> {
        self.signer_address.or_else(|| Self::default_authorized_signer(chain))
    }
}

/// The arguments for the L1 provider.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct L1ProviderArgs {
    /// The URL for the L1 RPC.
    #[arg(long = "l1.url", id = "l1_url", value_name = "L1_URL")]
    pub url: Option<reqwest::Url>,
    /// The compute units per second for the provider.
    #[arg(long = "l1.cups", id = "l1_compute_units_per_second", value_name = "L1_COMPUTE_UNITS_PER_SECOND", default_value_t = constants::PROVIDER_COMPUTE_UNITS_PER_SECOND)]
    pub compute_units_per_second: u64,
    /// The max amount of retries for the provider.
    #[arg(long = "l1.max-retries", id = "l1_max_retries", value_name = "L1_MAX_RETRIES", default_value_t = constants::L1_PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long = "l1.initial-backoff", id = "l1_initial_backoff", value_name = "L1_INITIAL_BACKOFF", default_value_t = constants::L1_PROVIDER_INITIAL_BACKOFF)]
    pub initial_backoff: u64,
    /// The logs query block range.
    #[arg(long = "l1.query-range", id = "l1_query_range", value_name = "L1_QUERY_RANGE", default_value_t = constants::LOGS_QUERY_BLOCK_RANGE)]
    pub logs_query_block_range: u64,
}

/// The arguments for the Beacon provider.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct BlobProviderArgs {
    /// The URLs for the beacon node blob provider.
    #[arg(
        long = "blob.beacon_node_urls",
        id = "blob_beacon_node_urls",
        value_name = "BLOB_BEACON_NODE_URLS"
    )]
    pub beacon_node_urls: Option<Vec<reqwest::Url>>,
    /// The URL for the s3 blob provider.
    #[arg(long = "blob.s3_url", id = "blob_s3_url", value_name = "BLOB_S3_URL")]
    pub s3_url: Option<reqwest::Url>,
    /// The URL for the anvil blob provider.
    #[arg(long = "blob.anvil_url", id = "blob_anvil_url", value_name = "BLOB_ANVIL_URL")]
    pub anvil_url: Option<reqwest::Url>,
    /// Enable the mock blob source.
    #[arg(long = "blob.mock")]
    pub mock: bool,
    /// The compute units per second for the provider.
    #[arg(long = "blob.cups", id = "blob_compute_units_per_second", value_name = "BLOB_COMPUTE_UNITS_PER_SECOND", default_value_t = constants::PROVIDER_COMPUTE_UNITS_PER_SECOND)]
    pub compute_units_per_second: u64,
    /// The max amount of retries for the provider.
    #[arg(long = "blob.max-retries", id = "blob_max_retries", value_name = "BLOB_MAX_RETRIES", default_value_t = constants::L1_PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long = "blob.initial-backoff", id = "blob_initial_backoff", value_name = "BLOB_INITIAL_BACKOFF", default_value_t = constants::L1_PROVIDER_INITIAL_BACKOFF)]
    pub initial_backoff: u64,
}

/// The arguments for the sequencer.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct SequencerArgs {
    /// Enable the scroll block sequencer.
    #[arg(long = "sequencer.enabled", default_value_t = false)]
    pub sequencer_enabled: bool,
    /// Whether the sequencer should start sequencing automatically on startup.
    #[arg(long = "sequencer.auto-start", default_value_t = false)]
    pub auto_start: bool,
    /// The block time for the sequencer.
    #[arg(long = "sequencer.block-time", id = "sequencer_block_time", value_name = "SEQUENCER_BLOCK_TIME", default_value_t = constants::DEFAULT_BLOCK_TIME)]
    pub block_time: u64,
    /// The payload building duration for the sequencer (milliseconds)
    #[arg(long = "sequencer.payload-building-duration", id = "sequencer_payload_building_duration", value_name = "SEQUENCER_PAYLOAD_BUILDING_DURATION", default_value_t = constants::DEFAULT_PAYLOAD_BUILDING_DURATION)]
    pub payload_building_duration: u64,
    /// The fee recipient for the sequencer.
    #[arg(long = "sequencer.fee-recipient", id = "sequencer_fee_recipient", value_name = "SEQUENCER_FEE_RECIPIENT", default_value_t = SCROLL_FEE_VAULT_ADDRESS)]
    pub fee_recipient: Address,
    /// L1 message inclusion mode: "finalized" or "depth:{number}"
    /// Examples: "finalized", "depth:10", "depth:6"
    #[arg(
        long = "sequencer.l1-inclusion-mode",
        id = "sequencer_l1_inclusion_mode",
        value_name = "MODE",
        default_value = "finalized:2",
        help = "L1 message inclusion mode. Use 'finalized' for finalized messages only, or 'depth:{number}' for block depth confirmation (e.g. 'depth:10')"
    )]
    pub l1_message_inclusion_mode: L1MessageInclusionMode,
    /// Enable empty blocks.
    #[arg(
        long = "sequencer.allow-empty-blocks",
        id = "sequencer_allow_empty_blocks",
        value_name = "SEQUENCER_ALLOW_EMPTY_BLOCKS",
        default_value_t = false
    )]
    pub allow_empty_blocks: bool,
    /// The maximum number of L1 messages to include per L2 block.
    #[arg(
        long = "sequencer.max-l1-messages",
        id = "sequencer_max_l1_messages",
        value_name = "SEQUENCER_MAX_L1_MESSAGES",
        help = "The maximum number of L1 messages to include per L2 block. If not set, defaults to the value specified in the chain config."
    )]
    pub max_l1_messages: Option<u64>,
}

/// The arguments for the signer.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct SignerArgs {
    /// Path to the file containing the signer's private key
    #[arg(
        long = "signer.key-file",
        value_name = "FILE_PATH",
        help = "Path to the hex-encoded private key file for the signer (optional 0x prefix). Mutually exclusive with --signer.aws-kms-key-id"
    )]
    pub key_file: Option<PathBuf>,

    /// AWS KMS Key ID for signing transactions
    #[arg(
        long = "signer.aws-kms-key-id",
        value_name = "KEY_ID",
        help = "AWS KMS Key ID for signing transactions. Mutually exclusive with --signer.key-file"
    )]
    pub aws_kms_key_id: Option<String>,

    /// The private key signer, if any.
    pub private_key: Option<PrivateKeySigner>,
}

/// The arguments for the rpc.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct RpcArgs {
    /// A boolean to represent if the rollup node rpc should be enabled.
    #[arg(long = "rpc.rollup-node", help = "Enable the rollup node RPC namespace")]
    pub enabled: bool,
}

impl SignerArgs {
    /// Create a signer based on the configured arguments
    pub async fn signer(
        &self,
        chain_id: u64,
    ) -> eyre::Result<Option<Box<dyn Signer + Send + Sync>>> {
        if let Some(key_file_path) = &self.key_file {
            // Load the private key from the file
            let key_content = fs::read_to_string(key_file_path)
                .map_err(|e| {
                    eyre::eyre!("Failed to read signer key file {}: {}", key_file_path.display(), e)
                })?
                .trim()
                .to_string();

            let hex_str = key_content.strip_prefix("0x").unwrap_or(&key_content);
            let key_bytes = hex::decode(hex_str).map_err(|e| {
                eyre::eyre!(
                    "Failed to decode hex private key from file {}: {}",
                    key_file_path.display(),
                    e
                )
            })?;

            // Create the private key signer
            let private_key_signer = PrivateKeySigner::from_slice(&key_bytes)
                .map_err(|e| eyre::eyre!("Failed to create signer from key file: {}", e))?
                .with_chain_id(Some(chain_id));

            tracing::info!(target: "scroll::node::args",
                "Created private key signer with address: {} for chain ID: {}",
                private_key_signer.address(),
                chain_id
            );

            Ok(Some(Box::new(private_key_signer)))
        } else if let Some(aws_kms_key_id) = &self.aws_kms_key_id {
            // Load AWS configuration
            let config_loader = aws_config::defaults(BehaviorVersion::latest());
            let config = config_loader.load().await;
            let kms_client = aws_sdk_kms::Client::new(&config);

            // Create the AWS KMS signer
            let aws_signer = AwsSigner::new(kms_client, aws_kms_key_id.clone(), Some(chain_id))
                .await
                .map_err(|e| eyre::eyre!("Failed to initialize AWS KMS signer: {}", e))?;

            tracing::info!(
                target: "scroll::node::args",
                "Created AWS KMS signer with address: {} for chain ID: {}",
                aws_signer.address(),
                chain_id
            );

            Ok(Some(Box::new(aws_signer)))
        } else if let Some(private_key) = &self.private_key {
            tracing::info!(target: "scroll::node::args", "Created private key signer with address: {} for chain ID: {}", private_key.address(), chain_id);
            let signer = private_key.clone().with_chain_id(Some(chain_id));
            Ok(Some(Box::new(signer)))
        } else {
            Ok(None)
        }
    }
}

/// The arguments for the sequencer.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct RollupNodeGasPriceOracleArgs {
    /// Minimum suggested priority fee (tip) in wei, default `100`
    #[arg(long, default_value_t = 100)]
    #[arg(long = "gpo.default-suggest-priority-fee", id = "default_suggest_priority_fee", value_name = "DEFAULT_SUGGEST_PRIORITY_FEE", default_value_t = constants::DEFAULT_SUGGEST_PRIORITY_FEE)]
    pub default_suggested_priority_fee: u64,
}

/// Returns the total difficulty constant for the given chain.
const fn td_constant(chain: Option<NamedChain>) -> U128 {
    match chain {
        Some(NamedChain::Scroll) => constants::SCROLL_MAINNET_TD_CONSTANT,
        Some(NamedChain::ScrollSepolia) => constants::SCROLL_SEPOLIA_TD_CONSTANT,
        _ => U128::ZERO, // Default to zero for other chains
    }
}

/// The L1 message queue index at which queue hashes should be computed .
const fn l1_v2_message_queue_start_index(chain: Option<NamedChain>) -> u64 {
    match chain {
        Some(NamedChain::Scroll) => constants::SCROLL_MAINNET_V2_MESSAGE_QUEUE_START_INDEX,
        Some(NamedChain::ScrollSepolia) => constants::SCROLL_SEPOLIA_V2_MESSAGE_QUEUE_START_INDEX,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_network_args_default_authorized_signer() {
        // Test Scroll mainnet
        let mainnet_signer =
            RollupNodeNetworkArgs::default_authorized_signer(Some(NamedChain::Scroll));
        assert_eq!(mainnet_signer, Some(constants::SCROLL_MAINNET_SIGNER));

        // Test Scroll Sepolia
        let sepolia_signer =
            RollupNodeNetworkArgs::default_authorized_signer(Some(NamedChain::ScrollSepolia));
        assert_eq!(sepolia_signer, Some(constants::SCROLL_SEPOLIA_SIGNER));

        // Test other chains
        let other_signer =
            RollupNodeNetworkArgs::default_authorized_signer(Some(NamedChain::Mainnet));
        assert_eq!(other_signer, None);

        // Test None chain
        let none_signer = RollupNodeNetworkArgs::default_authorized_signer(None);
        assert_eq!(none_signer, None);
    }

    #[test]
    fn test_network_args_effective_signer() {
        let custom_signer = Address::new([0x11; 20]);

        // Test with configured signer
        let network_args =
            RollupNodeNetworkArgs { signer_address: Some(custom_signer), ..Default::default() };
        assert_eq!(network_args.effective_signer(Some(NamedChain::Scroll)), Some(custom_signer));

        // Test without configured signer, fallback to default
        let network_args_default = RollupNodeNetworkArgs::default();
        assert_eq!(
            network_args_default.effective_signer(Some(NamedChain::Scroll)),
            Some(constants::SCROLL_MAINNET_SIGNER)
        );
        assert_eq!(
            network_args_default.effective_signer(Some(NamedChain::ScrollSepolia)),
            Some(constants::SCROLL_SEPOLIA_SIGNER)
        );
        assert_eq!(network_args_default.effective_signer(Some(NamedChain::Mainnet)), None);
    }

    #[test]
    fn test_validate_sequencer_enabled_without_any_signer_fails() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: None, private_key: None },
            database_args: RollupNodeDatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            chain_orchestrator_args: ChainOrchestratorArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            blob_provider_args: BlobProviderArgs::default(),
            network_args: RollupNodeNetworkArgs::default(),
            gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
            consensus_args: ConsensusArgs {
                algorithm: ConsensusAlgorithm::SystemContract,
                authorized_signer: None,
            },
            database: None,
            rpc_args: RpcArgs::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(
            "Either signer key file, AWS KMS key ID or private key is required when sequencer is enabled"
        ));
    }

    #[test]
    fn test_validate_sequencer_enabled_with_both_signers_fails() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs {
                key_file: Some(PathBuf::from("/path/to/key")),
                aws_kms_key_id: Some("key-id".to_string()),
                private_key: None,
            },
            database_args: RollupNodeDatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            chain_orchestrator_args: ChainOrchestratorArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            blob_provider_args: BlobProviderArgs::default(),
            network_args: RollupNodeNetworkArgs::default(),
            gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
            consensus_args: ConsensusArgs {
                algorithm: ConsensusAlgorithm::SystemContract,
                authorized_signer: None,
            },
            database: None,
            rpc_args: RpcArgs::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot specify more than one signer key source"));
    }

    #[test]
    fn test_validate_sequencer_enabled_with_key_file_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs {
                key_file: Some(PathBuf::from("/path/to/key")),
                aws_kms_key_id: None,
                private_key: None,
            },
            database_args: RollupNodeDatabaseArgs::default(),
            chain_orchestrator_args: ChainOrchestratorArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            blob_provider_args: BlobProviderArgs::default(),
            network_args: RollupNodeNetworkArgs::default(),
            gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
            consensus_args: ConsensusArgs::noop(),
            database: None,
            rpc_args: RpcArgs::default(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_sequencer_enabled_with_aws_kms_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs {
                key_file: None,
                aws_kms_key_id: Some("key-id".to_string()),
                private_key: None,
            },
            database_args: RollupNodeDatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            chain_orchestrator_args: ChainOrchestratorArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            blob_provider_args: BlobProviderArgs::default(),
            network_args: RollupNodeNetworkArgs::default(),
            gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
            consensus_args: ConsensusArgs::noop(),
            database: None,
            rpc_args: RpcArgs::default(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_sequencer_disabled_without_any_signer_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: false, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: None, private_key: None },
            database_args: RollupNodeDatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            chain_orchestrator_args: ChainOrchestratorArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            blob_provider_args: BlobProviderArgs::default(),
            network_args: RollupNodeNetworkArgs::default(),
            gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
            consensus_args: ConsensusArgs::noop(),
            database: None,
            rpc_args: RpcArgs::default(),
        };

        assert!(config.validate().is_ok());
    }
}
