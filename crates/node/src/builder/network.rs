use alloy_primitives::{Address, Signature, B256};
use reth_chainspec::EthChainSpec;
use reth_eth_wire_types::BasicNetworkPrimitives;
use reth_network::{
    config::NetworkMode,
    protocol::{RlpxSubProtocol, RlpxSubProtocols},
    transform::header::{HeaderResponseTransform, HeaderTransform},
    NetworkConfig, NetworkHandle, NetworkManager, PeersInfo,
};
use reth_node_api::TxTy;
use reth_node_builder::{components::NetworkBuilder, BuilderContext, FullNodeTypes};
use reth_node_types::NodeTypes;
use reth_primitives_traits::BlockHeader;
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_primitives::ScrollPrimitives;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use rollup_node_primitives::sig_encode_hash;
use rollup_node_signer::SignatureAsBytes;
use reth_primitives_traits::Header;
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_db::{Database, DatabaseOperations};
use std::{fmt, fmt::Debug, path::PathBuf, sync::Arc};
use tracing::{debug, info, trace, warn};

use crate::args::NetworkArgs;

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
            NetworkArgs::default_authorized_signer(chain_spec.chain().named())
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
        let handle = ctx.start_network(network, pool, Some(Arc::new(request_transform)));
        info!(target: "reth::cli", enode=%handle.local_node_record(), "P2P networking initialized");
        Ok(handle)
    }
}

/// Network primitive types used by Scroll networks.
type ScrollNetworkPrimitives =
    BasicNetworkPrimitives<ScrollPrimitives, scroll_alloy_consensus::ScrollPooledTransaction>;

/// Errors that can occur during signature validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HeaderTransformError {
    /// Invalid signature length (expected 65 bytes)
    InvalidSignature,
    /// Invalid signer (not authorized)
    InvalidSigner(Address),
    /// Signature recovery failed
    RecoveryFailed,
    /// Database operation failed
    DatabaseError(String),
}

impl fmt::Display for HeaderTransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "Invalid signature length, expected 65 bytes"),
            Self::InvalidSigner(signer) => write!(f, "Invalid signer, not authorized: {}", signer),
            Self::RecoveryFailed => write!(f, "Failed to recover signer from signature"),
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for HeaderTransformError {}

/// An implementation of a [`HeaderTransform`] for downloaded headers for Scroll.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub(crate) struct ScrollHeaderTransform<ChainSpec> {
    chain_spec: ChainSpec,
    db: Arc<Database>,
    signer: Option<Address>,
}

impl<ChainSpec: EthChainSpec + ScrollHardforks + Debug + Send + Sync + 'static>
    ScrollHeaderTransform<ChainSpec>
{
    /// Returns a new instance of the [`ScrollHeaderTransform`] from the provider chain spec.
    pub(crate) const fn new(
        chain_spec: ChainSpec,
        db: Arc<Database>,
        signer: Option<Address>,
    ) -> Self {
        Self { chain_spec, db, signer }
    }
}

impl<H: BlockHeader, ChainSpec: EthChainSpec + ScrollHardforks + Debug + Send + Sync>
    HeaderTransform<H> for ScrollHeaderTransform<ChainSpec>
{
    fn map(&self, mut header: H) -> H {
        if !self.chain_spec.is_euclid_v2_active_at_timestamp(header.timestamp()) {
            return header;
        }
        // TODO: remove this once we deprecated l2geth
        // Validate and process signature

        if let Err(err) = self.validate_and_store_signature(&mut header, self.signer) {
            debug!(
                target: "scroll::network::response_header_transform",
                "Header signature persistence failed, block number: {:?}, header hash: {:?}, error: {}",
                header.number(),
                sig_encode_hash(&header_to_alloy(&header)), err
            );
        }

        header
    }
}

impl<ChainSpec: ScrollHardforks + Debug + Send + Sync> ScrollHeaderTransform<ChainSpec> {
    fn validate_and_store_signature<H: BlockHeader>(
        &self,
        header: &mut H,
        authorized_signer: Option<Address>,
    ) -> Result<(), HeaderTransformError> {
        let signature_bytes = std::mem::take(header.extra_data_mut());
        let signature = parse_65b_signature(&signature_bytes)?;
        let hash = sig_encode_hash(&header_to_alloy(header));

        // Recover and verify signer
        recover_and_verify_signer(&signature, hash, authorized_signer)?;

        // Store signature in database
        persist_signature(self.db.clone(), hash, signature);

        Ok(())
    }
}

/// An implementation of a [`HeaderTransform`] for header request responses for Scroll.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub(crate) struct ScrollRequestHeaderTransform<ChainSpec> {
    chain_spec: ChainSpec,
    db: Arc<Database>,
    signer: Option<Address>,
}

impl<ChainSpec: EthChainSpec + ScrollHardforks + Debug + Send + Sync + 'static>
    ScrollRequestHeaderTransform<ChainSpec>
{
    /// Returns a new instance of the [`ScrollHeaderTransform`] from the provider chain spec.
    pub(crate) const fn new(
        chain_spec: ChainSpec,
        db: Arc<Database>,
        signer: Option<Address>,
    ) -> Self {
        Self { chain_spec, db, signer }
    }
}

#[async_trait::async_trait]
impl<H: BlockHeader, ChainSpec: EthChainSpec + ScrollHardforks + Debug + Send + Sync>
    HeaderResponseTransform<H> for ScrollRequestHeaderTransform<ChainSpec>
{
    async fn map(&self, mut header: H) -> H {
        if !self.chain_spec.is_euclid_v2_active_at_timestamp(header.timestamp()) {
            return header;
        }

        // read the signature from the rollup node.
        let block_hash = sig_encode_hash(&header_to_alloy(&header));

        let signature = self
            .db
            .get_signature(block_hash)
            .await
            .inspect_err(|e| {
                warn!(target: "scroll::network::request_header_transform",
                    "Failed to get block signature from database, block number: {:?}, header hash: {:?}, error: {}",
                    header.number(),
                    block_hash,
                    HeaderTransformError::DatabaseError(e.to_string())
                )
            })
            .ok()
            .flatten();

        // If we have a signature in the database and it matches configured signer then add it
        // to the extra data field
        if let Some(sig) = signature {
            if let Err(err) = recover_and_verify_signer(&sig, block_hash, self.signer) {
                warn!(
                target: "scroll::network::request_header_transform",
                    "Found invalid signature(different from the hardcoded signer) for block number: {:?}, header hash: {:?}, sig: {:?}, error: {}",
                    header.number(),
                    block_hash,
                    sig.to_string(),
                    err
                );
            } else {
                *header.extra_data_mut() = sig.sig_as_bytes().into();
            }
        }

        header
    }
}

/// Recover signer from signature and verify authorization.
fn recover_and_verify_signer(
    signature: &Signature,
    hash: B256,
    authorized_signer: Option<Address>,
) -> Result<Address, HeaderTransformError> {
    // Recover signer from signature
    let signer = reth_primitives_traits::crypto::secp256k1::recover_signer(signature, hash)
        .map_err(|_| HeaderTransformError::RecoveryFailed)?;

    // Verify signer is authorized
    if Some(signer) != authorized_signer {
        return Err(HeaderTransformError::InvalidSigner(signer));
    }

    Ok(signer)
}

/// Parse a canonical 65-byte secp256k1 signature: r (32) | s (32) | v (1).
fn parse_65b_signature(bytes: &[u8]) -> Result<Signature, HeaderTransformError> {
    if bytes.len() != 65 {
        return Err(HeaderTransformError::InvalidSignature);
    }

    let signature =
        Signature::from_raw(bytes).map_err(|_| HeaderTransformError::InvalidSignature)?;

    Ok(signature)
}

/// Run the async DB insert from sync code safely.
fn persist_signature(db: Arc<Database>, hash: B256, signature: Signature) {
    tokio::spawn(async move {
        trace!(
            "Persisting block signature to database, block hash: {:?}, sig: {:?}",
            hash,
            signature.to_string()
        );
        if let Err(e) = db.insert_signature(hash, signature).await {
            warn!(target: "scroll::network::header_transform", "Failed to store signature in database: {:?}", e);
        }
    });
}

/// Convert a generic `BlockHeader` to `alloy_consensus::Header`
fn header_to_alloy<H: BlockHeader>(header: &H) -> Header {
    Header {
        parent_hash: header.parent_hash(),
        ommers_hash: header.ommers_hash(),
        beneficiary: header.beneficiary(),
        state_root: header.state_root(),
        transactions_root: header.transactions_root(),
        receipts_root: header.receipts_root(),
        logs_bloom: header.logs_bloom(),
        difficulty: header.difficulty(),
        number: header.number(),
        gas_limit: header.gas_limit(),
        gas_used: header.gas_used(),
        timestamp: header.timestamp(),
        extra_data: header.extra_data().clone(),
        mix_hash: header.mix_hash().unwrap_or_default(),
        nonce: header.nonce().unwrap_or_default(),
        base_fee_per_gas: header.base_fee_per_gas(),
        withdrawals_root: header.withdrawals_root(),
        blob_gas_used: header.blob_gas_used(),
        excess_blob_gas: header.excess_blob_gas(),
        parent_beacon_block_root: header.parent_beacon_block_root(),
        requests_hash: header.requests_hash(),
    }
}
