use reth_network::{config::NetworkMode, NetworkConfig, NetworkManager, PeersInfo};
use reth_node_api::TxTy;
use reth_node_builder::{components::NetworkBuilder, BuilderContext, FullNodeTypes};
use reth_node_types::NodeTypes;
use reth_rpc_builder::config::RethRpcServerConfig;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_primitives::ScrollPrimitives;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use rollup_node_manager::{PoAConsensus, RollupNodeManager};
use scroll_alloy_provider::ScrollAuthEngineApiProvider;
use scroll_engine::{test_utils::NoopExecutionPayloadProvider, EngineDriver, ForkchoiceState};
use scroll_network::NetworkManager as ScrollNetworkManager;
use scroll_wire::{ProtocolHandler, ScrollWireConfig};
use tracing::info;

/// The network builder for the eth-wire to scroll-wire bridge.
#[derive(Debug, Default)]
pub struct ScrollBridgeNetworkBuilder;

impl<Node, Pool> NetworkBuilder<Node, Pool> for ScrollBridgeNetworkBuilder
where
    Node:
        FullNodeTypes<Types: NodeTypes<ChainSpec = ScrollChainSpec, Primitives = ScrollPrimitives>>,
    Pool: TransactionPool<
            Transaction: PoolTransaction<
                Consensus = TxTy<Node::Types>,
                Pooled = scroll_alloy_consensus::ScrollPooledTransaction,
            >,
        > + Unpin
        + 'static,
{
    type Primitives = reth_scroll_node::ScrollNetworkPrimitives;

    async fn build_network(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
    ) -> eyre::Result<reth_network::NetworkHandle<Self::Primitives>> {
        // Create a new block channel to bridge between eth-wire and scroll-wire protocols.
        let (new_block_tx, new_block_rx) = tokio::sync::mpsc::unbounded_channel();

        // Create a scroll-wire protocol handler.
        let (scroll_wire_handler, events) = ProtocolHandler::new(ScrollWireConfig::new(true));

        // Create the network configuration.
        let config = ctx.network_config()?;
        let mut config = NetworkConfig {
            network_mode: NetworkMode::Work,
            block_import: Box::new(super::BridgeBlockImport::new(new_block_tx.clone())),
            ..config
        };

        // Add the scroll-wire protocol handler to the network config.
        config.extra_protocols.push(scroll_wire_handler);

        // Create the network manager.
        let network = NetworkManager::<Self::Primitives>::builder(config).await?;
        let handle = ctx.start_network(network, pool);

        // Create the scroll network manager.
        let scroll_network_manager = ScrollNetworkManager::from_parts(handle.clone(), events);

        // Spawn the scroll network manager.
        let consensus = PoAConsensus::new(vec![]);
        let payload_provider = NoopExecutionPayloadProvider;

        let auth_port = ctx.config().rpc.auth_port;
        let auth_secret = ctx.config().rpc.auth_jwt_secret(ctx.config().datadir().jwt())?;

        let engine_api = ScrollAuthEngineApiProvider::new(
            auth_secret,
            format!("http://localhost:{auth_port}").parse()?,
        );
        let engine = EngineDriver::new(engine_api, payload_provider);

        let rollup_node_manager = RollupNodeManager::new(
            scroll_network_manager,
            engine,
            ForkchoiceState::genesis(
                ctx.config().chain.chain.try_into().expect("must be a named chain"),
            ),
            consensus,
            new_block_rx,
        );

        ctx.task_executor().spawn(rollup_node_manager);

        info!(target: "scroll::reth::cli", enode=%handle.local_node_record(), "P2P networking initialized");
        Ok(handle)
    }
}
