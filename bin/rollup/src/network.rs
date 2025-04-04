use crate::ScrollRollupNodeArgs;
use alloy_provider::ProviderBuilder;
use migration::MigratorTrait;
use reth_network::{config::NetworkMode, NetworkManager, PeersInfo};
use reth_node_api::TxTy;
use reth_node_builder::{components::NetworkBuilder, BuilderContext, FullNodeTypes};
use reth_node_types::NodeTypes;
use reth_rpc_builder::config::RethRpcServerConfig;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_primitives::ScrollPrimitives;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use rollup_node_indexer::Indexer;
use rollup_node_manager::{PoAConsensus, RollupNodeManager};
use rollup_node_providers::{DatabaseL1MessageProvider, OnlineBeaconClient, OnlineL1Provider};
use rollup_node_watcher::L1Watcher;
use scroll_alloy_provider::ScrollAuthEngineApiProvider;
use scroll_db::{Database, DatabaseConnectionProvider};
use scroll_engine::{test_utils::NoopExecutionPayloadProvider, EngineDriver, ForkchoiceState};
use scroll_network::NetworkManager as ScrollNetworkManager;
use scroll_wire::{ProtocolHandler, ScrollWireConfig};
use std::{path::PathBuf, sync::Arc};
use tracing::info;

/// The network builder for the eth-wire to scroll-wire bridge.
#[derive(Debug)]
pub struct ScrollRollupNetworkBuilder {
    config: ScrollRollupNodeArgs,
}

impl ScrollRollupNetworkBuilder {
    /// Returns a new [`ScrollRollupNetworkBuilder`] instance with the provided config.
    pub fn new(config: ScrollRollupNodeArgs) -> Self {
        Self { config }
    }
}

impl<Node, Pool> NetworkBuilder<Node, Pool> for ScrollRollupNetworkBuilder
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
        let (block_tx, block_rx) =
            if self.config.enable_eth_scroll_wire_bridge & self.config.enable_scroll_wire {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                (Some(tx), Some(rx))
            } else {
                (None, None)
            };

        // Create a scroll-wire protocol handler.
        let (scroll_wire_handler, events) = ProtocolHandler::new(ScrollWireConfig::new(true));

        // Create the network configuration.
        let mut config = ctx.network_config()?;
        config.network_mode = NetworkMode::Work;
        if let Some(tx) = block_tx {
            config.block_import = Box::new(super::BridgeBlockImport::new(tx.clone()))
        }

        // Add the scroll-wire protocol handler to the network config.
        if self.config.enable_scroll_wire {
            config.extra_protocols.push(scroll_wire_handler);
        }

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
            self.config.engine_api_url.unwrap_or(format!("http://localhost:{auth_port}").parse()?),
        );
        let engine = EngineDriver::new(engine_api, payload_provider);

        // Instantiate the database
        let database_path = if let Some(db_path) = self.config.database_path {
            db_path
        } else {
            PathBuf::from("sqlite://").join(ctx.config().datadir().db().join("scroll.db"))
        };
        let db = Database::new(database_path.to_str().unwrap()).await?;

        // Run the database migrations
        migration::Migrator::up(db.get_connection(), None).await?;

        // Wrap the database in an Arc
        let db = Arc::new(db);

        // Spawn the indexer
        let indexer = Indexer::new(db.clone());

        // Spawn the L1Watcher
        let l1_notification_rx = if let Some(l1_rpc_url) = self.config.l1_rpc_url {
            Some(L1Watcher::spawn(ProviderBuilder::new().on_http(l1_rpc_url), 20035952).await)
        } else {
            None
        };

        let beacon_client = OnlineBeaconClient::new_http(self.config.beacon_rpc_url.to_string());
        let l1_messages_provider = DatabaseL1MessageProvider::new(db.clone(), 0);
        let l1_provider = OnlineL1Provider::new(beacon_client, 100, l1_messages_provider).await;

        // Spawn the rollup node manager
        let rollup_node_manager = RollupNodeManager::new(
            scroll_network_manager,
            engine,
            l1_provider,
            l1_notification_rx,
            indexer,
            ForkchoiceState::genesis(
                ctx.config().chain.chain.try_into().expect("must be a named chain"),
            ),
            consensus,
            block_rx,
        );

        ctx.task_executor().spawn(rollup_node_manager);

        info!(target: "scroll::reth::cli", enode=%handle.local_node_record(), "P2P networking initialized");
        Ok(handle)
    }
}
