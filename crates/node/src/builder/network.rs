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
use reth_primitives_traits::{BlockHeader, Header};
use reth_scroll_chainspec::ScrollChainSpec;
use reth_scroll_primitives::ScrollPrimitives;
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use rollup_node_primitives::sig_encode_hash;
use rollup_node_signer::SignatureAsBytes;
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_db::{Database, DatabaseReadOperations, DatabaseWriteOperations};
use std::{fmt, fmt::Debug, sync::Arc};
use tracing::{debug, info, trace, warn};

use crate::args::RollupNodeNetworkArgs;

/// The network builder for Scroll.
#[derive(Debug)]
pub struct ScrollNetworkBuilder {
    /// Additional `RLPx` sub-protocols to be added to the network.
    scroll_sub_protocols: RlpxSubProtocols,
    /// A reference to the rollup-node `Database`.
    rollup_node_db: Arc<Database>,
    /// The address for which we should persist and serve signatures for.
    signer: Option<Address>,
}

impl ScrollNetworkBuilder {
    /// Create a new [`ScrollNetworkBuilder`] with provided rollup node database.
    pub fn new(rollup_node_db: Arc<Database>) -> Self {
        Self { scroll_sub_protocols: RlpxSubProtocols::default(), rollup_node_db, signer: None }
    }

    /// Add a scroll sub-protocol to the network builder.
    pub fn with_sub_protocol(mut self, protocol: RlpxSubProtocol) -> Self {
        self.scroll_sub_protocols.push(protocol);
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
        // get the header transform.
        let chain_spec = ctx.chain_spec();
        let authorized_signer = if self.signer.is_none() {
            RollupNodeNetworkArgs::default_authorized_signer(chain_spec.chain().named())
        } else {
            self.signer
        };
        let transform = ScrollHeaderTransform::new(
            chain_spec.clone(),
            self.rollup_node_db.clone(),
            authorized_signer,
        );
        let request_transform = ScrollRequestHeaderTransform::new(
            chain_spec,
            self.rollup_node_db.clone(),
            authorized_signer,
        );

        // set the network mode to work.
        let config = ctx.network_config()?;
        let config = NetworkConfig {
            network_mode: NetworkMode::Work,
            header_transform: Arc::new(transform),
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
    chain_spec: Arc<ChainSpec>,
    db: Arc<Database>,
    signer: Option<Address>,
}

impl<ChainSpec: EthChainSpec + ScrollHardforks + Debug + Send + Sync + 'static>
    ScrollHeaderTransform<ChainSpec>
{
    /// Returns a new instance of the [`ScrollHeaderTransform`] from the provider chain spec.
    pub(crate) const fn new(
        chain_spec: Arc<ChainSpec>,
        db: Arc<Database>,
        signer: Option<Address>,
    ) -> Self {
        Self { chain_spec, db, signer }
    }
}

#[async_trait::async_trait]
impl<H: BlockHeader, ChainSpec: EthChainSpec + ScrollHardforks + Debug + Send + Sync + 'static>
    HeaderTransform<H> for ScrollHeaderTransform<ChainSpec>
{
    async fn map(&self, mut headers: Vec<H>) -> Vec<H> {
        // TODO: remove this once we deprecated l2geth
        let signer = self.signer;
        let chain_spec = self.chain_spec.clone();

        // Process headers in a blocking task to avoid blocking the async runtime
        let (headers, signatures) = tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;

            let signatures: Vec<(B256, Signature)> = headers.par_iter_mut().filter_map(|header| {
                if !chain_spec.is_euclid_v2_active_at_timestamp(header.timestamp()) {
                    return None
                }

                let signature_bytes = std::mem::take(header.extra_data_mut());
                let signature = if let Ok(sig) = parse_65b_signature(&signature_bytes) {
                    sig
                } else {
                    debug!(
                            target: "scroll::network::header_transform",
                            "Header signature persistence skipped due to invalid signature, block number: {:?}, header hash: {:?}",
                            header.number(),
                            header.hash_slow(),

                    );
                    return None
                };

                // Recover and verify signer
                let hash = header.hash_slow();
                if let Err(err) = recover_and_verify_signer(&signature, header, signer) {
                    debug!(
                        target: "scroll::network::header_transform",
                        "Header signature persistence failed, block number: {:?}, header hash: {:?}, error: {}",
                        header.number(),
                        hash, err
                    );
                    return None;
                }

                Some((hash, signature))
            }
            ).collect();

            (headers, signatures)
        }).await.expect("header transform task panicked");

        // Store signatures in database
        trace!(
            target: "scroll::network::header_transform",
            count = signatures.len(),
            "Persisting block signatures to database",
        );
        if let Err(e) = self.db.insert_signatures(signatures).await {
            warn!(target: "scroll::network::header_transform", "Failed to store signatures in database: {:?}", e);
        }

        headers
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
        let hash = header.hash_slow();

        let signature = self.db.get_signature(hash).await.inspect_err(|e| {
            warn!(target: "scroll::network::request_header_transform",
                        "Failed to get block signature from database, block number: {:?}, header hash: {:?}, error: {}",
                        header.number(),
                    hash,
                        HeaderTransformError::DatabaseError(e.to_string())
                    )
        })
            .ok()
            .flatten();

        // If we have a signature in the database and it matches configured signer then add it
        // to the extra data field
        if let Some(sig) = signature {
            if let Err(err) = recover_and_verify_signer(&sig, &header, self.signer) {
                warn!(
                target: "scroll::network::request_header_transform",
                    "Found invalid signature(different from the hardcoded signer={:?}) for block number: {:?}, header hash: {:?}, sig: {:?}, error: {}",
                    self.signer,
                    header.number(),
                    hash,
                    sig.to_string(),
                    err
                );
            } else {
                *header.extra_data_mut() = sig.sig_as_bytes().into();
            }
        } else {
            debug!(
                target: "scroll::network::request_header_transform",
                "No signature found in database for block number: {:?}, header hash: {:?}",
                header.number(),
                hash,
            );
        }

        header
    }
}

/// Recover signer from signature and verify authorization.
fn recover_and_verify_signer<H: BlockHeader>(
    signature: &Signature,
    header: &H,
    authorized_signer: Option<Address>,
) -> Result<Address, HeaderTransformError> {
    let hash = sig_encode_hash(&header_to_alloy(header));

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
