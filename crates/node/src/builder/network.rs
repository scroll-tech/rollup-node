use alloy_primitives::{address, Address};
use reth_chainspec::{EthChainSpec, NamedChain};
use reth_eth_wire_types::BasicNetworkPrimitives;
use reth_network::{
    config::NetworkMode,
    protocol::{RlpxSubProtocol, RlpxSubProtocols},
    NetworkConfig, NetworkHandle, NetworkManager, PeersInfo,
};
use reth_node_api::TxTy;
use reth_node_builder::{components::NetworkBuilder, BuilderContext, FullNodeTypes};
use reth_node_types::NodeTypes;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_node::{ScrollHeaderTransform, ScrollRequestHeaderTransform};
use reth_scroll_primitives::ScrollPrimitives;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use scroll_db::Database;
use std::{fmt::Debug, path::PathBuf, sync::Arc};
use tracing::info;

/// The network builder for Scroll.
#[derive(Debug, Default)]
pub struct ScrollNetworkBuilder {
    /// Additional `RLPx` sub-protocols to be added to the network.
    scroll_sub_protocols: RlpxSubProtocols,
    /// A reference to the rollup-node `Database`.
    rollup_node_db_path: Option<PathBuf>,
    /// The address for which we should persist and serve signatures for.
    signer: Option<Address>,
}

impl ScrollNetworkBuilder {
    /// Create a new [`ScrollNetworkBuilder`] with default configuration.
    pub fn new() -> Self {
        Self {
            scroll_sub_protocols: RlpxSubProtocols::default(),
            rollup_node_db_path: None,
            signer: None,
        }
    }

    /// Add a scroll sub-protocol to the network builder.
    pub fn with_sub_protocol(mut self, protocol: RlpxSubProtocol) -> Self {
        self.scroll_sub_protocols.push(protocol);
        self
    }

    /// Add a scroll sub-protocol to the network builder.
    pub fn with_database_path(mut self, db_path: Option<PathBuf>) -> Self {
        self.rollup_node_db_path = db_path;
        self
    }

    /// Set the signer for which we will persist and serve signatures if included in block header
    /// extra data field.
    pub const fn with_signer(mut self, signer: Option<Address>) -> Self {
        self.signer = signer;
        self
    }
}

impl<Node, Pool> NetworkBuilder<Node, Pool> for ScrollNetworkBuilder
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
    type Network = NetworkHandle<ScrollNetworkPrimitives>;

    async fn build_network(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
    ) -> eyre::Result<Self::Network> {
        // initialize the rollup node database.
        let db_path = ctx.config().datadir().db();
        let database_path = if let Some(database_path) = self.rollup_node_db_path {
            database_path.to_string_lossy().to_string()
        } else {
            // append the path using strings as using `join(...)` overwrites "sqlite://"
            // if the path is absolute.
            let path = db_path.join("scroll.db?mode=rwc");
            "sqlite://".to_string() + &*path.to_string_lossy()
        };
        let db = Arc::new(Database::new(&database_path).await?);

        // get the header transform.
        let chain_spec = ctx.chain_spec();
        let authorized_signer = if self.signer.is_none() {
            match chain_spec.chain().named() {
                Some(NamedChain::Scroll) => Some(SCROLL_MAINNET_SIGNER),
                Some(NamedChain::ScrollSepolia) => Some(SCROLL_SEPOLIA_SIGNER),
                _ => None,
            }
        } else {
            self.signer
        };
        let transform =
            ScrollHeaderTransform::new(chain_spec.clone(), db.clone(), authorized_signer);
        let request_transform =
            ScrollRequestHeaderTransform::new(chain_spec, db.clone(), authorized_signer);

        // set the network mode to work.
        let config = ctx.network_config()?;
        let config = NetworkConfig {
            network_mode: NetworkMode::Work,
            header_transform: Box::new(transform),
            extra_protocols: self.scroll_sub_protocols,
            ..config
        };

        let network = NetworkManager::builder(config).await?;
        let handle = ctx.start_network(network, pool, Some(Box::new(request_transform)));
        info!(target: "reth::cli", enode=%handle.local_node_record(), "P2P networking initialized");
        Ok(handle)
    }
}

/// Network primitive types used by Scroll networks.
type ScrollNetworkPrimitives =
    BasicNetworkPrimitives<ScrollPrimitives, scroll_alloy_consensus::ScrollPooledTransaction>;

/// The correct signer address for Scroll mainnet and sepolia.
const SCROLL_MAINNET_SIGNER: Address = address!("0xD83C4892BB5aA241B63d8C4C134920111E142A20");
const SCROLL_SEPOLIA_SIGNER: Address = address!("0x687E0E85AD67ff71aC134CF61b65905b58Ab43b2");
