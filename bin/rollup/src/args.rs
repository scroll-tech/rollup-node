use std::path::PathBuf;

#[derive(Debug, clap::Args)]
pub struct ScrollBridgeNodeArgs {
    /// A bool to represent if new blocks should be bridged from the eth wire protocol to the
    /// scroll wire protocol.
    pub enable_eth_scroll_wire_bridge: bool,
    /// A bool that represents if the scroll wire protocol should be enabled.
    pub enable_scroll_wire: bool,
    /// Database path
    pub database_path: Option<PathBuf>,
}
