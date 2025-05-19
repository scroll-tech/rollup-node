use reth_node_api::FullNodeComponents;
use reth_node_builder::rpc::{RpcHandle, RpcHandleProvider};
use reth_rpc_eth_api::EthApiTypes;
use rollup_node_manager::RollupManagerHandle;

/// A handle for scroll addons, which includes handles for the rollup manager and RPC server.
#[derive(Debug, Clone)]
pub struct ScrollAddOnsHandle<Node: FullNodeComponents, EthApi: EthApiTypes> {
    /// The handle used to send commands to the rollup manager.
    pub rollup_manager_handle: RollupManagerHandle,
    /// The handle used to send commands to the RPC server.
    pub rpc_handle: RpcHandle<Node, EthApi>,
}

impl<Node: FullNodeComponents, EthApi: EthApiTypes> RpcHandleProvider<Node, EthApi>
    for ScrollAddOnsHandle<Node, EthApi>
{
    fn rpc_handle(&self) -> &RpcHandle<Node, EthApi> {
        &self.rpc_handle
    }
}
