use alloy_provider::ProviderBuilder;
use alloy_rpc_client::RpcClient;
use alloy_transport::layers::RetryBackoffLayer;
use migration::MigratorTrait;
use reth_network::{config::NetworkMode, NetworkManager, PeersInfo};
use reth_node_api::TxTy;
use reth_node_builder::{components::NetworkBuilder, BuilderContext, FullNodeTypes};
use reth_node_types::NodeTypes;
use reth_rpc_builder::config::RethRpcServerConfig;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_primitives::ScrollPrimitives;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use rollup_node_manager::{PoAConsensus, RollupNodeManager};
use rollup_node_providers::{beacon_provider, DatabaseL1MessageProvider, OnlineL1Provider};
use rollup_node_watcher::L1Watcher;
use scroll_alloy_provider::ScrollAuthEngineApiProvider;
use scroll_db::{Database, DatabaseConnectionProvider};
use scroll_engine::{test_utils::NoopExecutionPayloadProvider, EngineDriver, ForkchoiceState};
use scroll_network::NetworkManager as ScrollNetworkManager;
use scroll_wire::{ProtocolHandler, ScrollWireConfig};
use std::sync::Arc;
use tracing::info;

use crate::{
    constants::PROVIDER_BLOB_CACHE_SIZE, L1ProviderArgs, ScrollRollupNodeArgs,
    WATCHER_START_BLOCK_NUMBER,
};

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
            db_path.to_string_lossy().to_string()
        } else {
            // append the path using strings as using `join(...)` overwrites "sqlite://"
            // if the path is absolute.
            let path = ctx.config().datadir().db().join("scroll.db");
            "sqlite://".to_string() + &*path.to_string_lossy()
        };
        let db = Database::new(&database_path).await?;

        // Run the database migrations
        migration::Migrator::up(db.get_connection(), None).await?;

        // Wrap the database in an Arc
        let db = Arc::new(db);

        // Spawn the L1Watcher
        let l1_provider_args = self.config.l1_provider_args;
        let l1_notification_rx = if let Some(l1_rpc_url) = l1_provider_args.l1_rpc_url {
            let L1ProviderArgs { max_retries, initial_backoff, compute_units_per_second, .. } =
                l1_provider_args;
            let client = RpcClient::builder()
                .layer(RetryBackoffLayer::new(
                    max_retries,
                    initial_backoff,
                    compute_units_per_second,
                ))
                .http(l1_rpc_url);
            let provider = ProviderBuilder::new().on_client(client);
            Some(L1Watcher::spawn(provider, WATCHER_START_BLOCK_NUMBER).await)
        } else {
            None
        };

        // Construct the l1 provider.
        let beacon_provider = beacon_provider(l1_provider_args.beacon_rpc_url.to_string());
        let l1_messages_provider = DatabaseL1MessageProvider::new(db.clone(), 0);
        let l1_provider =
            OnlineL1Provider::new(beacon_provider, PROVIDER_BLOB_CACHE_SIZE, l1_messages_provider)
                .await;

        // Spawn the rollup node manager
        let rollup_node_manager = RollupNodeManager::new(
            scroll_network_manager,
            engine,
            l1_provider,
            db,
            l1_notification_rx,
            // initiating the safe and finalized block info with a null hash triggers a backfill
            // using the unsafe head at the EN.
            ForkchoiceState::default(),
            consensus,
            block_rx,
        );

        ctx.task_executor().spawn(rollup_node_manager);

        info!(target: "scroll::reth::cli", enode=%handle.local_node_record(), "P2P networking initialized");
        Ok(handle)
    }
}
