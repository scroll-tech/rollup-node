use crate::constants;
use alloy_primitives::Address;
use reth_scroll_chainspec::SCROLL_FEE_VAULT_ADDRESS;
use std::path::PathBuf;

/// A struct that represents the arguments for the rollup node.
#[derive(Debug, clap::Args)]
pub struct ScrollRollupNodeArgs {
    /// Whether the rollup node should be run in dev mode.
    #[arg(long)]
    pub dev: bool,
    /// A bool to represent if new blocks should be bridged from the eth wire protocol to the
    /// scroll wire protocol.
    #[arg(long, default_value_t = false)]
    pub enable_eth_scroll_wire_bridge: bool,
    /// A bool that represents if the scroll wire protocol should be enabled.
    #[arg(long, default_value_t = false)]
    pub enable_scroll_wire: bool,
    /// Database path
    #[arg(long)]
    pub database_path: Option<PathBuf>,
    /// The EngineAPI URL.
    #[arg(long)]
    pub engine_api_url: Option<reqwest::Url>,
    /// The provider arguments
    #[command(flatten)]
    pub l1_provider_args: L1ProviderArgs,
    /// The sequencer arguments
    #[command(flatten)]
    pub sequencer_args: Option<SequencerArgs>,
}

#[derive(Debug, clap::Args)]
pub struct L1ProviderArgs {
    /// The URL for the L1 RPC URL.
    #[arg(long)]
    pub l1_rpc_url: Option<reqwest::Url>,
    /// The URL for the Beacon RPC URL.
    #[arg(long)]
    pub beacon_rpc_url: Option<reqwest::Url>,
    /// The compute units per second for the provider.
    #[arg(long, default_value_t = constants::PROVIDER_COMPUTE_UNITS_PER_SECOND)]
    pub compute_units_per_second: u64,
    /// The max amount of retries for the provider.
    #[arg(long, default_value_t = constants::PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long, default_value_t = constants::PROVIDER_INITIAL_BACKOFF)]
    pub initial_backoff: u64,
}

#[derive(Debug, Clone, Default, clap::Args)]
#[group(requires_all = ["block_time", "payload_building_duration", "max_l1_messages_per_block", "fee_recipient"])]
pub struct SequencerArgs {
    /// The block time for the sequencer.
    #[arg(long, default_value_t = constants::DEFAULT_BLOCK_TIME, required = false)]
    pub scroll_block_time: u64,
    /// The payload building duration for the sequencer (milliseconds)
    #[arg(long, default_value_t = constants::DEFAULT_PAYLOAD_BUILDING_DURATION, required = false)]
    pub payload_building_duration: u64,
    /// The max L1 messages per block for the sequencer.
    #[arg(long, default_value_t = constants::DEFAULT_MAX_L1_MESSAGES_PER_BLOCK, required = false)]
    pub max_l1_messages_per_block: u64,
    /// The fee recipient for the sequencer.
    #[arg(long, default_value_t = SCROLL_FEE_VAULT_ADDRESS, required = false)]
    pub fee_recipient: Address,
}
