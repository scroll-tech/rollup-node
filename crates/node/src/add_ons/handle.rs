use reth_node_api::FullNodeComponents;
use reth_node_builder::rpc::{RpcHandle, RpcHandleProvider};
use reth_rpc_eth_api::EthApiTypes;
use rollup_node_manager::RollupManagerHandle;
#[cfg(feature = "test-utils")]
use {rollup_node_watcher::L1Notification, std::sync::Arc, tokio::sync::mpsc::Sender};

/// A handle for scroll addons, which includes handles for the rollup manager and RPC server.
#[derive(Debug, Clone)]
pub struct ScrollAddOnsHandle<Node: FullNodeComponents, EthApi: EthApiTypes> {
    /// The handle used to send commands to the rollup manager.
    pub rollup_manager_handle: RollupManagerHandle,
    /// The handle used to send commands to the RPC server.
    pub rpc_handle: RpcHandle<Node, EthApi>,
    /// An optional channel used to send `L1Watcher` notifications to the `RollupNodeManager`.
    #[cfg(feature = "test-utils")]
    pub l1_watcher_tx: Option<Sender<Arc<L1Notification>>>,
}

impl<Node: FullNodeComponents, EthApi: EthApiTypes> RpcHandleProvider<Node, EthApi>
    for ScrollAddOnsHandle<Node, EthApi>
{
    fn rpc_handle(&self) -> &RpcHandle<Node, EthApi> {
        &self.rpc_handle
    }
}
