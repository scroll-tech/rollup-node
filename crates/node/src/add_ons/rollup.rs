use crate::args::ScrollRollupNodeConfig;

use reth_chainspec::NamedChain;
use reth_network::NetworkProtocols;
use reth_network_api::FullNetwork;
use reth_node_api::{FullNodeTypes, NodeTypes};
use reth_node_builder::{rpc::RpcHandle, AddOnsContext, FullNodeComponents};
use reth_rpc_eth_api::EthApiTypes;
use reth_scroll_chainspec::{ChainConfig, ScrollChainConfig, ScrollChainSpec};
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_manager::RollupManagerHandle;
use rollup_node_watcher::L1Notification;
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_wire::ScrollWireEvent;
use std::sync::Arc;
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

/// Implementing the trait allows the type to return whether it is configured for dev chain.
#[auto_impl::auto_impl(Arc)]
pub trait IsDevChain {
    /// Returns true if the chain is a dev chain.
    fn is_dev_chain(&self) -> bool;
}

impl IsDevChain for ScrollChainSpec {
    fn is_dev_chain(&self) -> bool {
        let named: Result<NamedChain, _> = self.chain.try_into();
        named.is_ok_and(|n| matches!(n, NamedChain::Dev))
    }
}

/// The rollup node manager addon.
#[derive(Debug)]
pub struct RollupManagerAddOn {
    config: ScrollRollupNodeConfig,
    scroll_wire_event: UnboundedReceiver<ScrollWireEvent>,
}

impl RollupManagerAddOn {
    /// Create a new rollup node manager addon.
    pub const fn new(
        config: ScrollRollupNodeConfig,
        scroll_wire_event: UnboundedReceiver<ScrollWireEvent>,
    ) -> Self {
        Self { config, scroll_wire_event }
    }

    /// Launch the rollup node manager addon.
    pub async fn launch<N: FullNodeComponents, EthApi: EthApiTypes>(
        self,
        ctx: AddOnsContext<'_, N>,
        rpc: RpcHandle<N, EthApi>,
    ) -> eyre::Result<(RollupManagerHandle<N::Network>, Option<Sender<Arc<L1Notification>>>)>
    where
        <<N as FullNodeTypes>::Types as NodeTypes>::ChainSpec:
            ChainConfig<Config = ScrollChainConfig> + ScrollHardforks + IsDevChain,
        N::Network: NetworkProtocols + FullNetwork<Primitives = ScrollNetworkPrimitives>,
    {
        let (rnm, handle, l1_notification_tx) = self
            .config
            .build((&ctx).into(), self.scroll_wire_event, rpc.rpc_server_handles)
            .await?;
        ctx.node
            .task_executor()
            .spawn_critical_with_graceful_shutdown_signal("rollup_node_manager", |shutdown| {
                rnm.run_until_graceful_shutdown(shutdown)
            });
        Ok((handle, l1_notification_tx))
    }
}
