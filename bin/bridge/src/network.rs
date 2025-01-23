use network::{NetworkManager as ScrollNetworkManager, NoopBlockImport};
use reth_network::{
    config::NetworkMode, EthNetworkPrimitives, NetworkConfig, NetworkManager, PeersInfo,
};
use reth_node_api::TxTy;
use reth_node_builder::{components::NetworkBuilder, BuilderContext, FullNodeTypes};
use reth_node_types::NodeTypes;
use reth_primitives::{EthPrimitives, PooledTransaction};
use reth_scroll_chainspec::ScrollChainSpec;
use reth_tracing::tracing::info;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use scroll_wire::{ProtocolHandler, ScrollWireConfig};

/// The network builder for the eth-wire to scroll-wire bridge.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScrollBridgeNetworkBuilder;

impl<Node, Pool> NetworkBuilder<Node, Pool> for ScrollBridgeNetworkBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ScrollChainSpec, Primitives = EthPrimitives>>,
    Pool: TransactionPool<
            Transaction: PoolTransaction<Consensus = TxTy<Node::Types>, Pooled = PooledTransaction>,
        > + Unpin
        + 'static,
{
    type Primitives = EthNetworkPrimitives;

    async fn build_network(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
    ) -> eyre::Result<reth_network::NetworkHandle> {
        // Create a new block channel to bridge between eth-wire and scroll-wire protocols.
        let (new_block_tx, new_block_rx) = tokio::sync::mpsc::unbounded_channel();

        // Create a scroll-wire protocol handler.
        let (scroll_wire_handler, events) = ProtocolHandler::new(ScrollWireConfig::new(true));

        // Initialize the network manager.
        let mut config = NetworkConfig {
            network_mode: NetworkMode::Work,
            block_import: Box::new(super::BridgeBlockImport::new(new_block_tx)),
            ..ctx.network_config()?
        };

        // Add the scroll-wire protocol handler to the network manager.
        config.extra_protocols.push(scroll_wire_handler);

        // Create the network manager.
        let network = NetworkManager::builder(config).await?;
        let handle = ctx.start_network(network, pool);

        // Create the scroll network manager.
        let scroll_wire_manager =
            ScrollNetworkManager::from_parts(handle.clone(), Box::new(NoopBlockImport), events)
                .with_new_block_source(new_block_rx);

        // Spawn the scroll network manager.
        ctx.task_executor().spawn(scroll_wire_manager);

        info!(target: "reth::cli", enode=%handle.local_node_record(), "P2P networking initialized");
        Ok(handle)
    }
}
