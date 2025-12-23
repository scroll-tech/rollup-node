#[cfg(feature = "test-utils")]
use crate::test_utils::l1_helpers::L1WatcherMock;
use reth_network_api::FullNetwork;
use reth_node_api::FullNodeComponents;
use reth_node_builder::rpc::{RpcHandle, RpcHandleProvider};
use reth_rpc_eth_api::EthApiTypes;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_chain_orchestrator::ChainOrchestratorHandle;

/// A handle for scroll addons, which includes handles for the rollup manager and RPC server.
#[derive(Debug, Clone)]
pub struct ScrollAddOnsHandle<
    Node: FullNodeComponents<Network: FullNetwork<Primitives = ScrollNetworkPrimitives>>,
    EthApi: EthApiTypes,
> {
    /// The handle used to send commands to the rollup manager.
    pub rollup_manager_handle: ChainOrchestratorHandle<Node::Network>,
    /// The handle used to send commands to the RPC server.
    pub rpc_handle: RpcHandle<Node, EthApi>,
    /// An optional channel used to send `L1Watcher` notifications to the `RollupNodeManager`.
    #[cfg(feature = "test-utils")]
    pub l1_watcher_tx: Option<L1WatcherMock>,
}

impl<
        Node: FullNodeComponents<Network: FullNetwork<Primitives = ScrollNetworkPrimitives>>,
        EthApi: EthApiTypes,
    > RpcHandleProvider<Node, EthApi> for ScrollAddOnsHandle<Node, EthApi>
{
    fn rpc_handle(&self) -> &RpcHandle<Node, EthApi> {
        &self.rpc_handle
    }
}
