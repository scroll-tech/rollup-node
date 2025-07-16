pub(crate) mod blob;
pub(crate) mod message;
pub(crate) mod system_contract;

use crate::{l1::message::L1MessageProvider, BlobProvider};
use std::sync::Arc;

use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use alloy_transport::{RpcError, TransportErrorKind};
use rollup_node_primitives::L1MessageEnvelope;
use scroll_db::DatabaseError;

/// An instance of the trait can be used to provide L1 data.
pub trait L1Provider: BlobProvider + L1MessageProvider {}
impl<T> L1Provider for T where T: BlobProvider + L1MessageProvider {}

/// An error occurring at the [`L1Provider`].
#[derive(Debug, thiserror::Error)]
pub enum L1ProviderError {
    /// Error at the beacon provider.
    #[error("Beacon provider error: {0}")]
    BeaconProvider(#[from] reqwest::Error),
    /// Invalid timestamp for slot.
    #[error("invalid block timestamp: genesis {0}, provided {1}")]
    InvalidBlockTimestamp(u64, u64),
    /// Database error.
    #[error(transparent)]
    Database(#[from] DatabaseError),
    /// L1 RPC error.
    #[error(transparent)]
    Rpc(#[from] RpcError<TransportErrorKind>),
    /// Other error.
    #[error("{0}")]
    Other(&'static str),
}

/// An implementation of the [`L1Provider`] trait which caches blobs.
#[derive(Debug, Clone)]
pub struct FullL1Provider<L1MP, BP> {
    /// The blob provider.
    l1_blob_provider: BP,
    /// The L1 message provider
    l1_message_provider: L1MP,
}

impl<L1MP, BP> FullL1Provider<L1MP, BP>
where
    BP: BlobProvider,
{
    /// Returns a new [`FullL1Provider`] from the provided [`L1BlobProvider`], blob capacity and
    /// [`L1MessageProvider`].
    pub async fn new(l1_blob_provider: BP, l1_message_provider: L1MP) -> Self {
        Self { l1_blob_provider, l1_message_provider }
    }
}

#[async_trait::async_trait]
impl<L1MP: Sync + Send, BP: BlobProvider> BlobProvider for FullL1Provider<L1MP, BP> {
    /// Returns the requested blob corresponding to the passed hash.
    async fn blob(
        &self,
        block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        // query the blobs from the L1 blob provider.
        Ok(self.l1_blob_provider.blob(block_timestamp, hash).await?)
    }
}

#[async_trait::async_trait]
impl<L1MP: L1MessageProvider + Send + Sync, BP: Sync + Send> L1MessageProvider
    for FullL1Provider<L1MP, BP>
{
    type Error = <L1MP>::Error;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageEnvelope>, Self::Error> {
        self.l1_message_provider.get_l1_message_with_block_number().await
    }

    fn set_queue_index_cursor(&self, index: u64) {
        self.l1_message_provider.set_queue_index_cursor(index);
    }

    async fn set_hash_cursor(&self, hash: B256) {
        self.l1_message_provider.set_hash_cursor(hash).await
    }

    fn increment_cursor(&self) {
        self.l1_message_provider.increment_cursor()
    }
}
