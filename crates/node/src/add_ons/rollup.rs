use crate::{
    args::{L1ProviderArgs, ScrollRollupNodeConfig},
    constants::PROVIDER_BLOB_CACHE_SIZE,
};
use alloy_primitives::Sealable;
use alloy_provider::ProviderBuilder;
use alloy_rpc_client::RpcClient;
use alloy_signer_local::PrivateKeySigner;
use alloy_transport::layers::RetryBackoffLayer;
use reth_chainspec::EthChainSpec;
use reth_network::{protocol::IntoRlpxSubProtocol, NetworkProtocols};
use reth_network_api::{block::EthWireBlockListenerProvider, FullNetwork};
use reth_node_api::{FullNodeTypes, NodeTypes};
use reth_node_builder::{rpc::RpcHandle, AddOnsContext, FullNodeComponents};
use reth_rpc_eth_api::EthApiTypes;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_manager::{
    Consensus, NoopConsensus, PoAConsensus, RollupManagerHandle, RollupNodeManager,
};
use rollup_node_primitives::{ConsensusUpdate, NodeConfig};
use rollup_node_providers::{
    beacon_provider, AlloyExecutionPayloadProvider, DatabaseL1MessageProvider, OnlineL1Provider,
    SystemContractProvider,
};
use rollup_node_sequencer::Sequencer;
use rollup_node_signer::Signer;
use rollup_node_watcher::{L1Notification, L1Watcher};
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_provider::ScrollAuthApiEngineClient;
use scroll_db::{Database, DatabaseConnectionProvider};
use scroll_engine::{EngineDriver, ForkchoiceState};
use scroll_migration::MigratorTrait;
use scroll_network::ScrollNetworkManager;
use scroll_wire::{ScrollWireConfig, ScrollWireProtocolHandler};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc::Sender;

// Replace `Scroll` with the actual network type you use if it's generic

/// The rollup node manager addon.
#[derive(Debug)]
pub struct RollupManagerAddOn {
    config: ScrollRollupNodeConfig,
}

impl RollupManagerAddOn {
    /// Create a new rollup node manager addon.
    pub const fn new(config: ScrollRollupNodeConfig) -> Self {
        Self { config }
    }

    /// Launch the rollup node manager addon.
    pub async fn launch<N: FullNodeComponents, EthApi: EthApiTypes>(
        self,
        ctx: AddOnsContext<'_, N>,
        rpc: RpcHandle<N, EthApi>,
    ) -> eyre::Result<(RollupManagerHandle, Option<Sender<Arc<L1Notification>>>)>
    where
        <<N as FullNodeTypes>::Types as NodeTypes>::ChainSpec: ScrollHardforks,
        N::Network: NetworkProtocols + FullNetwork<Primitives = ScrollNetworkPrimitives>,
    {
        // Instantiate the network manager
        let (scroll_wire_handler, events) =
            ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));
        ctx.node.network().add_rlpx_sub_protocol(scroll_wire_handler.into_rlpx_sub_protocol());
        let scroll_network_manager =
            ScrollNetworkManager::from_parts(ctx.node.network().clone(), events);

        // Get the rollup node config.
        let node_config = Arc::new(NodeConfig::from_named_chain(
            ctx.config.chain.chain().named().expect("expected named chain"),
        ));

        // Create the engine api client.
        let engine_api = ScrollAuthApiEngineClient::new(rpc.rpc_server_handles.auth.http_client());

        // Get a provider
        let provider = self.config.l1_provider_args.url.clone().map(|url| {
            let L1ProviderArgs { max_retries, initial_backoff, compute_units_per_second, .. } =
                self.config.l1_provider_args;
            let client = RpcClient::builder()
                .layer(RetryBackoffLayer::new(
                    max_retries,
                    initial_backoff,
                    compute_units_per_second,
                ))
                .http(url);
            ProviderBuilder::new().connect_client(client)
        });

        // Get a payload provider
        let payload_provider = (ctx.config.rpc.http || self.config.test).then_some({
            rpc.rpc_server_handles
                .rpc
                .new_http_provider_for()
                .map(Arc::new)
                .map(AlloyExecutionPayloadProvider::new)
                .expect("failed to create payload provider")
        });

        let fcs = ForkchoiceState::head_from_genesis(ctx.config.chain.genesis_header().hash_slow());
        let engine = EngineDriver::new(
            Arc::new(engine_api),
            payload_provider,
            fcs,
            true,
            Duration::from_millis(self.config.sequencer_args.payload_building_duration),
        );

        // Instantiate the database
        let database_path = if let Some(db_path) = self.config.database_path {
            db_path.to_string_lossy().to_string()
        } else {
            // append the path using strings as using `join(...)` overwrites "sqlite://"
            // if the path is absolute.
            let path = ctx.config.datadir().db().join("scroll.db?mode=rwc");
            "sqlite://".to_string() + &*path.to_string_lossy()
        };
        let db = Database::new(&database_path).await?;

        // Run the database migrations
        scroll_migration::Migrator::up(db.get_connection(), None).await?;

        // Wrap the database in an Arc
        let db = Arc::new(db);

        // Create the consensus.
        let consensus: Box<dyn Consensus> = if self.config.test {
            Box::new(NoopConsensus::default())
        } else {
            let mut poa = PoAConsensus::new([]);
            if let Some(ref provider) = provider {
                let signer = provider
                    .authorized_signer(node_config.address_book.system_contract_address)
                    .await?;
                poa.update_config(&ConsensusUpdate::AuthorizedSigner(signer));
            }
            Box::new(poa)
        };

        let (l1_notification_tx, l1_notification_rx) =
            if let Some(provider) = provider.filter(|_| !self.config.test) {
                // Spawn the L1Watcher
                (None, Some(L1Watcher::spawn(provider, node_config).await))
            } else {
                // Create a channel for L1 notifications that we can use to inject L1 messages for
                // testing
                #[cfg(feature = "test-utils")]
                {
                    let (tx, rx) = tokio::sync::mpsc::channel(1000);
                    (Some(tx), Some(rx))
                }

                #[cfg(not(feature = "test-utils"))]
                {
                    (None, None)
                }
            };

        // Construct the l1 provider.
        let l1_messages_provider = DatabaseL1MessageProvider::new(db.clone(), 0);
        let l1_provider = if let Some(url) = self.config.beacon_provider_args.url {
            let beacon_provider = beacon_provider(url.to_string());
            let l1_provider = OnlineL1Provider::new(
                beacon_provider,
                PROVIDER_BLOB_CACHE_SIZE,
                l1_messages_provider.clone(),
            )
            .await;
            Some(l1_provider)
        } else {
            None
        };

        // Construct the Sequencer.
        let (sequencer, block_time) = if self.config.sequencer_args.sequencer_enabled {
            let args = &self.config.sequencer_args;
            let sequencer = Sequencer::new(
                Arc::new(l1_messages_provider),
                args.fee_recipient,
                args.max_l1_messages_per_block,
                0,
                0,
            );
            (Some(sequencer), (args.block_time != 0).then_some(args.block_time))
        } else {
            (None, None)
        };

        // Instantiate the eth wire listener
        let eth_wire_listener = self
            .config
            .network_args
            .enable_eth_scroll_wire_bridge
            .then_some(ctx.node.network().eth_wire_block_listener().await?);

        // Instantiate the signer
        let signer = self.config.test.then_some(Signer::spawn(PrivateKeySigner::random()).await);

        // Spawn the rollup node manager
        let rnm = RollupNodeManager::new(
            scroll_network_manager,
            engine,
            l1_provider,
            db,
            l1_notification_rx,
            consensus,
            ctx.config.chain.clone(),
            eth_wire_listener,
            sequencer,
            signer,
            block_time,
        );
        Ok((rnm, l1_notification_tx))
    }
}
