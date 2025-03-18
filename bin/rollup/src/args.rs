use std::path::PathBuf;

/// A struct that represents the arguments for the rollup node.
#[derive(Debug, clap::Args)]
pub struct ScrollRollupNodeArgs {
    /// A bool to represent if new blocks should be bridged from the eth wire protocol to the
    /// scroll wire protocol.
    pub enable_eth_scroll_wire_bridge: bool,
    /// A bool that represents if the scroll wire protocol should be enabled.
    pub enable_scroll_wire: bool,
    /// Database path
    pub database_path: Option<PathBuf>,
    /// The URL for the L1 RPC URL.
    pub l1_rpc_url: Option<reqwest::Url>,
    /// The EngineAPI URL.
    pub engine_api_url: Option<reqwest::Url>,
}
