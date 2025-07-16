use crate::{BlobProvider, L1ProviderError};
use std::sync::Arc;

use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;

/// Mocks all calls to the beacon chain.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct MockBeaconProvider;

#[async_trait::async_trait]
impl BlobProvider for MockBeaconProvider {
    async fn blob(
        &self,
        _block_timestamp: u64,
        _hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        Ok(None)
    }
}
