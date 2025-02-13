use reth_network::{config::NetworkMode, NetworkConfig, NetworkManager, PeersInfo};
use reth_node_api::TxTy;
use reth_node_builder::{components::NetworkBuilder, BuilderContext, FullNodeTypes};
use reth_node_types::NodeTypes;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_primitives::ScrollPrimitives;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use scroll_network::{NetworkManager as ScrollNetworkManager, NoopBlockImport};
use scroll_wire::{ProtocolHandler, ScrollWireConfig};
use tracing::info;

/// The network builder for the eth-wire to scroll-wire bridge.
#[derive(Debug, Default)]
pub struct ScrollBridgeNetworkBuilder {
    block_import:
        Option<Box<dyn reth_network::import::BlockImport<reth_scroll_primitives::ScrollBlock>>>,
}

impl ScrollBridgeNetworkBuilder {
    /// Creates a new [`ScrollBridgeNetworkBuilder`] with the provided block import.
    #[cfg(feature = "test-utils")]
    pub fn new(
        block_import: Box<
            dyn reth_network::import::BlockImport<reth_scroll_primitives::ScrollBlock>,
        >,
    ) -> Self {
        Self { block_import: Some(block_import) }
    }
}

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
            block_import: Box::new(super::BridgeBlockImport::new(
                new_block_tx,
                self.block_import.unwrap_or(config.block_import),
            )),
            ..config
        };

        // Add the scroll-wire protocol handler to the network config.
        config.extra_protocols.push(scroll_wire_handler);

        // Create the network manager.
        let network = NetworkManager::<Self::Primitives>::builder(config).await?;
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
