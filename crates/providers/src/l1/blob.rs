use crate::L1ProviderError;
use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use std::sync::Arc;

/// An instance of the trait can be used to fetch L1 blob data.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait L1BlobProvider {
    /// Returns corresponding blob data for the provided hash.
    async fn blob(
        &self,
        block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError>;
}
