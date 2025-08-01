use crate::{
    add_ons::IsDevChain,
    constants::{self},
};
use std::{fs, path::PathBuf, sync::Arc, time::Duration};

use alloy_primitives::{hex, Address};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_client::RpcClient;
use alloy_signer::Signer;
use alloy_signer_aws::AwsSigner;
use alloy_signer_local::PrivateKeySigner;
use alloy_transport::layers::RetryBackoffLayer;
use aws_sdk_kms::config::BehaviorVersion;
use reth_chainspec::EthChainSpec;
use reth_network::NetworkProtocols;
use reth_network_api::FullNetwork;
use reth_node_builder::rpc::RethRpcServerHandles;
use reth_node_core::primitives::BlockHeader;
use reth_scroll_chainspec::SCROLL_FEE_VAULT_ADDRESS;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_manager::{
    Consensus, NoopConsensus, RollupManagerHandle, RollupNodeManager, SystemContractConsensus,
};
use rollup_node_primitives::{BlockInfo, NodeConfig};
use rollup_node_providers::{
    BlobSource, DatabaseL1MessageProvider, FullL1Provider, L1MessageProvider, L1Provider,
    SystemContractProvider,
};
use rollup_node_sequencer::{L1MessageInclusionMode, Sequencer};
use rollup_node_watcher::{L1Notification, L1Watcher};
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_network::Scroll;
use scroll_alloy_provider::{ScrollAuthApiEngineClient, ScrollEngineApi};
use scroll_db::{Database, DatabaseConnectionProvider, DatabaseOperations};
use scroll_engine::{genesis_hash_from_chain_spec, EngineDriver, ForkchoiceState};
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
    /// Database args
    #[command(flatten)]
    pub database_args: DatabaseArgs,
    /// Engine driver args.
    #[command(flatten)]
    pub engine_driver_args: EngineDriverArgs,
    /// The beacon provider arguments.
    #[command(flatten)]
    pub beacon_provider_args: BeaconProviderArgs,
    /// The L1 provider arguments
    #[command(flatten)]
    pub l1_provider_args: L1ProviderArgs,
    /// The sequencer arguments
    #[command(flatten)]
    pub sequencer_args: SequencerArgs,
    /// The network arguments
    #[command(flatten)]
    pub network_args: NetworkArgs,
    /// The signer arguments
    #[command(flatten)]
    pub signer_args: SignerArgs,
    /// The gas price oracle args
    #[command(flatten)]
    pub gas_price_oracle_args: GasPriceOracleArgs,
}

impl ScrollRollupNodeConfig {
    /// Validate that either signer key file or AWS KMS key ID is provided when sequencer is enabled
    pub fn validate(&self) -> Result<(), String> {
        if !self.test && self.sequencer_args.sequencer_enabled {
            if self.signer_args.key_file.is_none() && self.signer_args.aws_kms_key_id.is_none() {
                return Err("Either signer key file or AWS KMS key ID is required when sequencer is enabled".to_string());
            }
            if self.signer_args.key_file.is_some() && self.signer_args.aws_kms_key_id.is_some() {
                return Err("Cannot specify both signer key file and AWS KMS key ID".to_string());
            }
        }
        Ok(())
    }
}

impl ScrollRollupNodeConfig {
    /// Consumes the [`ScrollRollupNodeConfig`] and builds a [`RollupNodeManager`].
    pub async fn build<
        N: FullNetwork<Primitives = ScrollNetworkPrimitives> + NetworkProtocols,
        CS: ScrollHardforks
            + EthChainSpec<Header: BlockHeader>
            + IsDevChain
            + Clone
            + Send
            + Sync
            + 'static,
    >(
        self,
        network: N,
        events: UnboundedReceiver<ScrollWireEvent>,
        rpc_server_handles: RethRpcServerHandles,
        chain_spec: CS,
        db_path: PathBuf,
    ) -> eyre::Result<(
        RollupNodeManager<
            N,
            impl ScrollEngineApi,
            impl Provider<Scroll> + Clone,
            impl L1Provider + Clone,
            impl L1MessageProvider,
            impl ScrollHardforks + EthChainSpec<Header: BlockHeader> + IsDevChain + Clone + 'static,
        >,
        RollupManagerHandle<N>,
        Option<Sender<Arc<L1Notification>>>,
    )> {
        // Instantiate the network manager
        let scroll_network_manager = ScrollNetworkManager::from_parts(network.clone(), events);

        // Get the rollup node config.
        let named_chain = chain_spec.chain().named().expect("expected named chain");
        let node_config = Arc::new(NodeConfig::from_named_chain(named_chain));

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

        // Get a provider to the execution layer.
        let l2_provider = rpc_server_handles
            .rpc
            .new_http_provider_for()
            .map(Arc::new)
            .expect("failed to create payload provider");

        // Instantiate the database
        let database_path = if let Some(database_path) = self.database_args.path {
            database_path.to_string_lossy().to_string()
        } else {
            // append the path using strings as using `join(...)` overwrites "sqlite://"
            // if the path is absolute.
            let path = db_path.join("scroll.db?mode=rwc");
            "sqlite://".to_string() + &*path.to_string_lossy()
        };
        let db = Database::new(&database_path).await?;

        // Run the database migrations
        named_chain
            .migrate(db.get_connection(), self.test)
            .await
            .expect("failed to perform migration");

        // Wrap the database in an Arc
        let db = Arc::new(db);

        let chain_spec_fcs = || {
            ForkchoiceState::head_from_chain_spec(chain_spec.clone())
                .expect("failed to derive forkchoice state from chain spec")
        };
        let mut fcs = ForkchoiceState::head_from_provider(l2_provider.clone())
            .await
            .unwrap_or_else(chain_spec_fcs);

        let chain_spec = Arc::new(chain_spec.clone());

        // On startup we replay the latest batch of blocks from the database as such we set the safe
        // block hash to the latest block hash associated with the previous consolidated
        // batch in the database.
        let (startup_safe_block, l1_start_block_number) =
            db.prepare_on_startup(chain_spec.genesis_hash()).await?;
        if let Some(block_info) = startup_safe_block {
            fcs.update_safe_block_info(block_info);
        } else {
            fcs.update_safe_block_info(BlockInfo {
                hash: genesis_hash_from_chain_spec(chain_spec.clone()).unwrap(),
                number: 0,
            });
        }

        let engine = EngineDriver::new(
            Arc::new(engine_api),
            chain_spec.clone(),
            Some(l2_provider),
            fcs,
            self.engine_driver_args.sync_at_startup && !self.test && !chain_spec.is_dev_chain(),
            self.engine_driver_args.en_sync_trigger,
            Duration::from_millis(self.sequencer_args.payload_building_duration),
        );

        // Create the consensus.
        let consensus: Box<dyn Consensus> = if let Some(ref provider) = l1_provider {
            let signer = provider
                .authorized_signer(node_config.address_book.system_contract_address)
                .await?;
            Box::new(SystemContractConsensus::new(signer))
        } else {
            Box::new(NoopConsensus::default())
        };

        let (l1_notification_tx, l1_notification_rx): (Option<Sender<Arc<L1Notification>>>, _) =
            if let Some(provider) = l1_provider.filter(|_| !self.test) {
                // Determine the start block number for the L1 watcher
                (None, Some(L1Watcher::spawn(provider, l1_start_block_number, node_config).await))
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
        let l1_messages_provider = DatabaseL1MessageProvider::new(db.clone(), 0);
        let blob_provider = self
            .beacon_provider_args
            .blob_source
            .provider(self.beacon_provider_args.url)
            .await
            .expect("failed to construct L1 blob provider");
        let l1_provider = FullL1Provider::new(blob_provider, l1_messages_provider.clone()).await;

        // Construct the Sequencer.
        let (sequencer, block_time) = if self.sequencer_args.sequencer_enabled {
            let args = &self.sequencer_args;
            let sequencer = Sequencer::new(
                Arc::new(l1_messages_provider),
                args.fee_recipient,
                args.max_l1_messages_per_block,
                0,
                self.sequencer_args.l1_message_inclusion_mode,
            );
            (Some(sequencer), (args.block_time != 0).then_some(args.block_time))
        } else {
            (None, None)
        };

        // Instantiate the eth wire listener
        let eth_wire_listener = self
            .network_args
            .enable_eth_scroll_wire_bridge
            .then_some(network.eth_wire_block_listener().await?);

        // Instantiate the signer
        let signer = if self.test {
            // Use a random private key signer for testing
            Some(rollup_node_signer::Signer::spawn(PrivateKeySigner::random()))
        } else {
            // Use the signer configured by SignerArgs
            let chain_id = chain_spec.chain().id();
            self.signer_args.signer(chain_id).await?.map(rollup_node_signer::Signer::spawn)
        };

        // Spawn the rollup node manager
        let (rnm, handle) = RollupNodeManager::new(
            scroll_network_manager,
            engine,
            l1_provider,
            db,
            l1_notification_rx,
            consensus,
            chain_spec,
            eth_wire_listener,
            sequencer,
            signer,
            block_time,
        );
        Ok((rnm, handle, l1_notification_tx))
    }
}

/// The database arguments.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct DatabaseArgs {
    /// Database path
    #[arg(long)]
    pub path: Option<PathBuf>,
}

/// The engine driver args.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct EngineDriverArgs {
    /// The amount of block difference between the EN and the latest block received from P2P
    /// at which the engine driver triggers optimistic sync.
    #[arg(long = "engine.en-sync-trigger", default_value_t = constants::BLOCK_GAP_TRIGGER)]
    pub en_sync_trigger: u64,
    /// Whether the engine driver should try to sync at start up.
    #[arg(long = "engine.sync-at-startup", num_args=0..=1, default_value_t = true)]
    pub sync_at_startup: bool,
}

/// The network arguments.
#[derive(Debug, Clone, clap::Args)]
pub struct NetworkArgs {
    /// A bool to represent if new blocks should be bridged from the eth wire protocol to the
    /// scroll wire protocol.
    #[arg(long = "network.bridge", default_value_t = true)]
    pub enable_eth_scroll_wire_bridge: bool,
    /// A bool that represents if the scroll wire protocol should be enabled.
    #[arg(long = "network.scroll-wire", default_value_t = true)]
    pub enable_scroll_wire: bool,
    /// The URL for the Sequencer RPC. (can be both HTTP and WS)
    #[arg(
        long = "network.sequencer-url",
        id = "network_sequencer_url",
        value_name = "NETWORK_SEQUENCER_URL"
    )]
    pub sequencer_url: Option<String>,
}

impl Default for NetworkArgs {
    fn default() -> Self {
        Self { enable_eth_scroll_wire_bridge: true, enable_scroll_wire: true, sequencer_url: None }
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
    #[arg(long = "l1.max-retries", id = "l1_max_retries", value_name = "L1_MAX_RETRIES", default_value_t = constants::PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long = "l1.initial-backoff", id = "l1_initial_backoff", value_name = "L1_INITIAL_BACKOFF", default_value_t = constants::PROVIDER_INITIAL_BACKOFF)]
    pub initial_backoff: u64,
}

/// The arguments for the Beacon provider.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct BeaconProviderArgs {
    /// The URL for the Beacon chain.
    #[arg(long = "beacon.url", id = "beacon_url", value_name = "BEACON_URL")]
    pub url: Option<reqwest::Url>,
    /// The blob source for the provider.
    #[arg(
        long = "beacon.blob-source",
        id = "beacon_blob_source",
        value_name = "BEACON_BLOB_SOURCE",
        default_value = "mock"
    )]
    pub blob_source: BlobSource,
    /// The compute units per second for the provider.
    #[arg(long = "beacon.cups", id = "beacon_compute_units_per_second", value_name = "BEACON_COMPUTE_UNITS_PER_SECOND", default_value_t = constants::PROVIDER_COMPUTE_UNITS_PER_SECOND)]
    pub compute_units_per_second: u64,
    /// The max amount of retries for the provider.
    #[arg(long = "beacon.max-retries", id = "beacon_max_retries", value_name = "BEACON_MAX_RETRIES", default_value_t = constants::PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long = "beacon.initial-backoff", id = "beacon_initial_backoff", value_name = "BEACON_INITIAL_BACKOFF", default_value_t = constants::PROVIDER_INITIAL_BACKOFF)]
    pub initial_backoff: u64,
}

/// The arguments for the sequencer.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct SequencerArgs {
    /// Enable the scroll block sequencer.
    #[arg(long = "sequencer.enabled", default_value_t = false)]
    pub sequencer_enabled: bool,
    /// The block time for the sequencer.
    #[arg(long = "sequencer.block-time", id = "sequencer_block_time", value_name = "SEQUENCER_BLOCK_TIME", default_value_t = constants::DEFAULT_BLOCK_TIME)]
    pub block_time: u64,
    /// The payload building duration for the sequencer (milliseconds)
    #[arg(long = "sequencer.payload-building-duration", id = "sequencer_payload_building_duration", value_name = "SEQUENCER_PAYLOAD_BUILDING_DURATION", default_value_t = constants::DEFAULT_PAYLOAD_BUILDING_DURATION)]
    pub payload_building_duration: u64,
    /// The max L1 messages per block for the sequencer.
    #[arg(long = "sequencer.max-l1-messages-per-block", id = "sequencer_max_l1_messages_per_block", value_name = "SEQUENCER_MAX_L1_MESSAGES_PER_BLOCK", default_value_t = constants::DEFAULT_MAX_L1_MESSAGES_PER_BLOCK)]
    pub max_l1_messages_per_block: u64,
    /// The fee recipient for the sequencer.
    #[arg(long = "sequencer.fee-recipient", id = "sequencer_fee_recipient", value_name = "SEQUENCER_FEE_RECIPIENT", default_value_t = SCROLL_FEE_VAULT_ADDRESS)]
    pub fee_recipient: Address,
    /// L1 message inclusion mode: "finalized" or "depth:{number}"
    /// Examples: "finalized", "depth:10", "depth:6"
    #[arg(
        long = "sequencer.l1-inclusion-mode",
        id = "sequencer_l1_inclusion_mode",
        value_name = "MODE",
        default_value = "finalized",
        help = "L1 message inclusion mode. Use 'finalized' for finalized messages only, or 'depth:{number}' for block depth confirmation (e.g. 'depth:10')"
    )]
    pub l1_message_inclusion_mode: L1MessageInclusionMode,
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

            tracing::info!(
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
                "Created AWS KMS signer with address: {} for chain ID: {}",
                aws_signer.address(),
                chain_id
            );

            Ok(Some(Box::new(aws_signer)))
        } else {
            Ok(None)
        }
    }
}

/// The arguments for the sequencer.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct GasPriceOracleArgs {
    /// Minimum suggested priority fee (tip) in wei, default `100`
    #[arg(long, default_value_t = 100)]
    #[arg(long = "gpo.default-suggest-priority-fee", id = "default_suggest_priority_fee", value_name = "DEFAULT_SUGGEST_PRIORITY_FEE", default_value_t = constants::DEFAULT_SUGGEST_PRIORITY_FEE)]
    pub default_suggested_priority_fee: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_validate_sequencer_enabled_without_any_signer_fails() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: None },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
            gas_price_oracle_args: GasPriceOracleArgs::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(
            "Either signer key file or AWS KMS key ID is required when sequencer is enabled"
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
            },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
            gas_price_oracle_args: GasPriceOracleArgs::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Cannot specify both signer key file and AWS KMS key ID"));
    }

    #[test]
    fn test_validate_sequencer_enabled_with_key_file_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs {
                key_file: Some(PathBuf::from("/path/to/key")),
                aws_kms_key_id: None,
            },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
            gas_price_oracle_args: GasPriceOracleArgs::default(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_sequencer_enabled_with_aws_kms_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: Some("key-id".to_string()) },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
            gas_price_oracle_args: GasPriceOracleArgs::default(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_test_mode_without_any_signer_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: true,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: None },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
            gas_price_oracle_args: GasPriceOracleArgs::default(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_sequencer_disabled_without_any_signer_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: false, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: None },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
            gas_price_oracle_args: GasPriceOracleArgs::default(),
        };

        assert!(config.validate().is_ok());
    }
}
