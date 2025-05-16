use crate::constants;
use alloy_primitives::Address;
use reth_scroll_chainspec::SCROLL_FEE_VAULT_ADDRESS;
use std::path::PathBuf;

/// A struct that represents the arguments for the rollup node.
#[derive(Debug, Clone, clap::Args)]
pub struct ScrollRollupNodeConfig {
    /// Whether the rollup node should be run in test mode.
    #[arg(long)]
    pub test: bool,
    /// A bool that represents whether optimistic sync of the EN should be performed.
    /// This is a temp solution and should be removed when implementing issue #23.
    #[arg(long, default_value_t = false)]
    pub optimistic_sync: bool,
    /// Database path
    #[arg(long)]
    pub database_path: Option<PathBuf>,
    /// The `EngineAPI` URL.
    #[arg(long)]
    pub engine_api_url: Option<reqwest::Url>,
    /// The beacon provider arguments.
    #[command(flatten)]
    pub beacon_provider_args: BeaconProviderArgs,
    /// The L1 provider arguments
    #[command(flatten)]
    pub l1_provider_args: L1ProviderArgs,
    /// The L2 provider arguments
    #[command(flatten)]
    pub l2_provider_args: L2ProviderArgs,
    /// The sequencer arguments
    #[command(flatten)]
    pub sequencer_args: SequencerArgs,
    /// The network arguments
    #[command(flatten)]
    pub network_args: NetworkArgs,
}

/// The network arguments.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct NetworkArgs {
    /// A bool to represent if new blocks should be bridged from the eth wire protocol to the
    /// scroll wire protocol.
    #[arg(long = "eth-scroll-bridge.enabled", default_value_t = true)]
    pub enable_eth_scroll_wire_bridge: bool,
    /// A bool that represents if the scroll wire protocol should be enabled.
    #[arg(long = "scroll-wire.enabled", default_value_t = true)]
    pub enable_scroll_wire: bool,
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

/// The arguments for the L2 provider.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct L2ProviderArgs {
    /// The compute units per second for the provider.
    #[arg(long = "l2.cups",  id = "l2_compute_units_per_second", value_name = "L2_COMPUTE_UNITS_PER_SECOND", default_value_t = constants::PROVIDER_COMPUTE_UNITS_PER_SECOND)]
    pub compute_units_per_second: u64,
    /// The max amount of retries for the provider.
    #[arg(long = "l2.max-retries", id = "l2_max_retries", value_name = "L2_MAX_RETRIES", default_value_t = constants::PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long = "l2.initial-backoff", id = "l2_initial_back_off", value_name = "L2_INITIAL_BACKOFF", default_value_t = constants::PROVIDER_INITIAL_BACKOFF)]
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
}
