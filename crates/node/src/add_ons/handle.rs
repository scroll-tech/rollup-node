use reth_network_api::FullNetwork;
use reth_node_api::FullNodeComponents;
use reth_node_builder::rpc::{RpcHandle, RpcHandleProvider};
use reth_rpc_eth_api::EthApiTypes;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_chain_orchestrator::ChainOrchestratorHandle;
#[cfg(feature = "test-utils")]
use tokio::sync::mpsc::UnboundedReceiver;
#[cfg(feature = "test-utils")]
use tokio::sync::Mutex;
#[cfg(feature = "test-utils")]
use {rollup_node_watcher::L1Notification, std::sync::Arc, tokio::sync::mpsc::Sender};

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
    pub l1_watcher_tx: Option<Sender<Arc<L1Notification>>>,
    /// An optional channel used to receive commands from the `RollupNodeManager` to the
    /// `L1Watcher`.
    #[cfg(feature = "test-utils")]
    pub l1_watcher_command_rx: Arc<Mutex<UnboundedReceiver<rollup_node_watcher::L1WatcherCommand>>>,
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
