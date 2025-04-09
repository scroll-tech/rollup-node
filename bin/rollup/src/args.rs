use crate::constants;
use std::path::PathBuf;

/// A struct that represents the arguments for the rollup node.
#[derive(Debug, clap::Args)]
pub struct ScrollRollupNodeArgs {
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
}

#[derive(Debug, clap::Args)]
pub struct L1ProviderArgs {
    /// The URL for the L1 RPC URL.
    #[arg(long)]
    pub l1_rpc_url: Option<reqwest::Url>,
    /// The URL for the Beacon RPC URL.
    #[arg(long)]
    pub beacon_rpc_url: reqwest::Url,
    /// The compute units per second for the provider.
    #[arg(long)]
    pub compute_units_per_second: u64,
    /// The max amount of retries for the provider.
    #[arg(long, default_value_t = constants::PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long, default_value_t = constants::PROVIDER_INITIAL_BACKOFF)]
    pub initial_backoff: u64,
}
