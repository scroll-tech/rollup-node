//! The [`ScrollRollupNodeAddOns`] implementation for the Scroll rollup node.

use super::args::ScrollRollupNodeConfig;
use reth_evm::{ConfigureEvm, EvmFactory, EvmFactoryFor};
use reth_network::NetworkProtocols;
use reth_network_api::FullNetwork;
use reth_node_api::{AddOnsContext, NodeAddOns};
use reth_node_builder::{
    rpc::{
        BasicEngineApiBuilder, EngineValidatorAddOn, EngineValidatorBuilder, EthApiBuilder,
        RethRpcAddOns, RpcAddOns, RpcHandle,
    },
    FullNodeComponents,
};
use reth_node_types::NodeTypes;
use reth_revm::context::TxEnv;
use reth_rpc_eth_types::error::FromEvmError;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_engine_primitives::ScrollEngineTypes;
use reth_scroll_evm::ScrollNextBlockEnvAttributes;
use reth_scroll_node::{
    ScrollEngineValidator, ScrollEngineValidatorBuilder, ScrollNetworkPrimitives, ScrollStorage,
};
use reth_scroll_primitives::ScrollPrimitives;
use reth_scroll_rpc::{eth::ScrollEthApiBuilder, ScrollEthApi, ScrollEthApiError};
use scroll_alloy_evm::ScrollTransactionIntoTxEnv;

mod handle;
pub use handle::ScrollAddonsHandle;

mod rollup;
use rollup::RollupManagerAddon;

/// Add-ons for the Scroll follower node.
#[derive(Debug)]
pub struct ScrollRollupNodeAddOns<N>
where
    N: FullNodeComponents,
    ScrollEthApiBuilder: EthApiBuilder<N>,
{
    /// Rpc add-ons responsible for launching the RPC servers and instantiating the RPC handlers
    /// and eth-api.
    pub rpc_add_ons: RpcAddOns<
        N,
        ScrollEthApiBuilder,
        ScrollEngineValidatorBuilder,
        BasicEngineApiBuilder<ScrollEngineValidatorBuilder>,
    >,

    /// Rollup manager addon responsible for managing the components of the rollup node.
    pub rollup_manager_addon: RollupManagerAddon,
}
impl<N> ScrollRollupNodeAddOns<N>
where
    N: FullNodeComponents,
    ScrollEthApiBuilder: EthApiBuilder<N>,
{
    /// Create a new instance of [`ScrollRollupNodeAddOns`].
    pub fn new(config: ScrollRollupNodeConfig) -> Self {
        let rpc_add_ons = RpcAddOns::default();
        let rollup_manager_addon = RollupManagerAddon::new(config);
        Self { rpc_add_ons, rollup_manager_addon }
    }
}
impl<N> NodeAddOns<N> for ScrollRollupNodeAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ScrollChainSpec,
            Primitives = ScrollPrimitives,
            Storage = ScrollStorage,
            Payload = ScrollEngineTypes,
        >,
        Evm: ConfigureEvm<NextBlockEnvCtx = ScrollNextBlockEnvAttributes>,
        Network: NetworkProtocols + FullNetwork<Primitives = ScrollNetworkPrimitives>,
    >,
    ScrollEthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = ScrollTransactionIntoTxEnv<TxEnv>>,
{
    type Handle = ScrollAddonsHandle<N, ScrollEthApi<N>>;

    async fn launch_add_ons(
        self,
        ctx: reth_node_api::AddOnsContext<'_, N>,
    ) -> eyre::Result<Self::Handle> {
        let Self { rpc_add_ons, rollup_manager_addon: rollup_node_manager_addon } = self;
        let rpc_handle: RpcHandle<N, ScrollEthApi<N>> =
            rpc_add_ons.launch_add_ons_with(ctx.clone(), |_, _, _| Ok(())).await?;
        // TODO: modify handle type to have a handle for both rpc and rollup node manager.
        let rollup_node_handle =
            rollup_node_manager_addon.launch(ctx.clone(), rpc_handle.clone()).await?;
        Ok(ScrollAddonsHandle { rollup_manager_handle: rollup_node_handle, rpc_handle })
    }
}

impl<N> RethRpcAddOns<N> for ScrollRollupNodeAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ScrollChainSpec,
            Primitives = ScrollPrimitives,
            Storage = ScrollStorage,
            Payload = ScrollEngineTypes,
        >,
        Evm: ConfigureEvm<NextBlockEnvCtx = ScrollNextBlockEnvAttributes>,
        Network: NetworkProtocols + FullNetwork<Primitives = ScrollNetworkPrimitives>,
    >,
    ScrollEthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = ScrollTransactionIntoTxEnv<TxEnv>>,
{
    type EthApi = ScrollEthApi<N>;

    fn hooks_mut(&mut self) -> &mut reth_node_builder::rpc::RpcHooks<N, Self::EthApi> {
        self.rpc_add_ons.hooks_mut()
    }
}

impl<N> EngineValidatorAddOn<N> for ScrollRollupNodeAddOns<N>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ScrollChainSpec,
            Primitives = ScrollPrimitives,
            Payload = ScrollEngineTypes,
        >,
    >,
    ScrollEthApiBuilder: EthApiBuilder<N>,
{
    type Validator = ScrollEngineValidator;

    async fn engine_validator(&self, ctx: &AddOnsContext<'_, N>) -> eyre::Result<Self::Validator> {
        ScrollEngineValidatorBuilder.build(ctx).await
    }
}
