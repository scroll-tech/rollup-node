//! The [`ScrollRollupNodeAddOns`] implementation for the Scroll rollup node.

use super::args::ScrollRollupNodeConfig;
use crate::constants;

use reth_evm::{ConfigureEngineEvm, EvmFactory, EvmFactoryFor};
use reth_network::NetworkProtocols;
use reth_network_api::FullNetwork;
use reth_node_api::{AddOnsContext, NodeAddOns, PayloadTypes};
use reth_node_builder::{
    rpc::{
        BasicEngineApiBuilder, BasicEngineValidatorBuilder, EngineValidatorAddOn, EthApiBuilder,
        Identity, RethRpcAddOns, RethRpcMiddleware, RpcAddOns,
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
    ScrollEngineValidatorBuilder, ScrollNetworkPrimitives, ScrollNodeTypes, ScrollStorage,
};
use reth_scroll_primitives::ScrollPrimitives;
use reth_scroll_rpc::{eth::ScrollEthApiBuilder, ScrollEthApiError};
use scroll_alloy_evm::ScrollTransactionIntoTxEnv;
use scroll_wire::ScrollWireEvent;

mod handle;
pub use handle::ScrollAddOnsHandle;

mod rpc;
pub use rpc::{RollupNodeExtApiServer, RollupNodeRpcExt};
#[cfg(test)]
pub use rpc::RollupNodeExtApiClient;

mod rollup;
pub use rollup::IsDevChain;
use rollup::RollupManagerAddOn;
use tokio::sync::mpsc::UnboundedReceiver;

/// Add-ons for the Scroll follower node.
#[derive(Debug)]
pub struct ScrollRollupNodeAddOns<N, RpcMiddleware = Identity>
where
    N: FullNodeComponents<Types: ScrollNodeTypes>,
    ScrollEthApiBuilder: EthApiBuilder<N>,
{
    /// Rpc add-ons responsible for launching the RPC servers and instantiating the RPC handlers
    /// and eth-api.
    pub rpc_add_ons: RpcAddOns<
        N,
        ScrollEthApiBuilder,
        ScrollEngineValidatorBuilder,
        BasicEngineApiBuilder<ScrollEngineValidatorBuilder>,
        BasicEngineValidatorBuilder<ScrollEngineValidatorBuilder>,
        RpcMiddleware,
    >,

    /// Rollup manager addon responsible for managing the components of the rollup node.
    pub rollup_manager_addon: RollupManagerAddOn,
}

impl<N> ScrollRollupNodeAddOns<N>
where
    N: FullNodeComponents<Types: ScrollNodeTypes>,
    ScrollEthApiBuilder: EthApiBuilder<N>,
{
    /// Create a new instance of [`ScrollRollupNodeAddOns`].
    pub fn new(
        config: ScrollRollupNodeConfig,
        scroll_wire_event: UnboundedReceiver<ScrollWireEvent>,
    ) -> Self {
        let rpc_add_ons = RpcAddOns::new(
            ScrollEthApiBuilder::default()
                .with_min_suggested_priority_fee(
                    config.gas_price_oracle_args.default_suggested_priority_fee,
                )
                .with_payload_size_limit(constants::DEFAULT_PAYLOAD_SIZE_LIMIT)
                .with_sequencer(config.network_args.sequencer_url.clone()),
            ScrollEngineValidatorBuilder::default(),
            BasicEngineApiBuilder::default(),
            BasicEngineValidatorBuilder::default(),
            Identity::new(),
        );
        let rollup_manager_addon = RollupManagerAddOn::new(config, scroll_wire_event);
        Self { rpc_add_ons, rollup_manager_addon }
    }
}

impl<N, RpcMiddleware> ScrollRollupNodeAddOns<N, RpcMiddleware>
where
    N: FullNodeComponents<Types: ScrollNodeTypes>,
    ScrollEthApiBuilder: EthApiBuilder<N>,
{
    /// Sets the provided middleware for the rollup node addons.
    pub fn with_middleware<T>(self, middleware: T) -> ScrollRollupNodeAddOns<N, T> {
        let rpc_add_ons = self.rpc_add_ons.with_rpc_middleware(middleware);
        ScrollRollupNodeAddOns { rpc_add_ons, rollup_manager_addon: self.rollup_manager_addon }
    }
}

impl<N, RpcMiddleware> NodeAddOns<N> for ScrollRollupNodeAddOns<N, RpcMiddleware>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ScrollChainSpec,
            Primitives = ScrollPrimitives,
            Storage = ScrollStorage,
            Payload = ScrollEngineTypes,
        >,
        Evm: ConfigureEngineEvm<
            <<N::Types as NodeTypes>::Payload as PayloadTypes>::ExecutionData,
            NextBlockEnvCtx = ScrollNextBlockEnvAttributes,
        >,
        Network: FullNetwork<Primitives = ScrollNetworkPrimitives> + NetworkProtocols,
    >,
    ScrollEthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = ScrollTransactionIntoTxEnv<TxEnv>>,
    RpcMiddleware: RethRpcMiddleware,
{
    type Handle = ScrollAddOnsHandle<N, <ScrollEthApiBuilder as EthApiBuilder<N>>::EthApi>;

    async fn launch_add_ons(self, ctx: AddOnsContext<'_, N>) -> eyre::Result<Self::Handle> {
        let Self { mut rpc_add_ons, rollup_manager_addon: rollup_node_manager_addon } = self;
        rpc_add_ons.eth_api_builder.with_propagate_local_transactions(
            !ctx.config.txpool.no_local_transactions_propagation,
        );

        let (tx, rx) = tokio::sync::oneshot::channel();
        let rollup_node_rpc_ext = RollupNodeRpcExt::<N::Network>::new(rx);
        rpc_add_ons = rpc_add_ons.extend_rpc_modules(move |ctx| {
            ctx.modules.merge_configured(rollup_node_rpc_ext.into_rpc())?;
            Ok(())
        });

        let rpc_handle = rpc_add_ons.launch_add_ons_with(ctx.clone(), |_| Ok(())).await?;
        let (rollup_manager_handle, l1_watcher_tx) =
            rollup_node_manager_addon.launch(ctx.clone(), rpc_handle.clone()).await?;

        tx.send(rollup_manager_handle.clone())
            .map_err(|_| eyre::eyre!("failed to send rollup manager handle"))?;

        Ok(ScrollAddOnsHandle {
            rollup_manager_handle,
            rpc_handle,
            #[cfg(feature = "test-utils")]
            l1_watcher_tx,
        })
    }
}

impl<N, RpcMiddleware> RethRpcAddOns<N> for ScrollRollupNodeAddOns<N, RpcMiddleware>
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = ScrollChainSpec,
            Primitives = ScrollPrimitives,
            Storage = ScrollStorage,
            Payload = ScrollEngineTypes,
        >,
        Evm: ConfigureEngineEvm<
            <<N::Types as NodeTypes>::Payload as PayloadTypes>::ExecutionData,
            NextBlockEnvCtx = ScrollNextBlockEnvAttributes,
        >,
        Network: FullNetwork<Primitives = ScrollNetworkPrimitives> + NetworkProtocols,
    >,
    ScrollEthApiError: FromEvmError<N::Evm>,
    EvmFactoryFor<N::Evm>: EvmFactory<Tx = ScrollTransactionIntoTxEnv<TxEnv>>,
    RpcMiddleware: RethRpcMiddleware,
{
    type EthApi = <ScrollEthApiBuilder as EthApiBuilder<N>>::EthApi;

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
        Evm: ConfigureEngineEvm<
            <<N::Types as NodeTypes>::Payload as PayloadTypes>::ExecutionData,
            NextBlockEnvCtx = ScrollNextBlockEnvAttributes,
        >,
        Network: FullNetwork<Primitives = ScrollNetworkPrimitives> + NetworkProtocols,
    >,
    ScrollEthApiBuilder: EthApiBuilder<N>,
{
    type ValidatorBuilder = BasicEngineValidatorBuilder<ScrollEngineValidatorBuilder>;

    fn engine_validator_builder(&self) -> Self::ValidatorBuilder {
        EngineValidatorAddOn::engine_validator_builder(&self.rpc_add_ons)
    }
}
