use crate::{BlobProvider, L1ProviderError};
use std::sync::Arc;

use alloy_eips::eip4844::Blob;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_client::RpcClient;

/// An implementation of a blob provider client using anvil.
#[derive(Debug, Clone)]
pub struct AnvilBlobProvider {
    /// The inner rpc client.
    pub inner: RpcClient,
}

impl AnvilBlobProvider {
    /// Creates a new [`AnvilBlobProvider`] from the provided url.
    pub fn new_http(url: reqwest::Url) -> Self {
        Self { inner: RpcClient::new_http(url) }
    }
}

#[async_trait::async_trait]
impl BlobProvider for AnvilBlobProvider {
    async fn blob(
        &self,
        _block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        // Request blob data from Anvil - it returns hex-encoded bytes
        let blob_data: Option<Bytes> = self.inner.request("anvil_getBlobByHash", (hash,)).await?;

        match blob_data {
            Some(data) => {
                // Convert bytes to Blob type (same as S3BlobProvider does)
                let blob = Blob::try_from(data.as_ref())
                    .map_err(|_| L1ProviderError::Other("Invalid blob data from Anvil"))?;
                Ok(Some(Arc::new(blob)))
            }
            None => Ok(None),
        }
    }
}
