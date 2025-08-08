use crate::{l1::blob::BLOB_SIZE, BlobProvider, L1ProviderError};
use reqwest::{Client, Url};
use std::sync::Arc;

use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;

/// An implementation of a blob provider client using S3.
#[derive(Debug, Clone)]
pub struct S3BlobProvider {
    /// The base URL for the S3 service.
    pub base_url: Url,
    /// HTTP client for making requests.
    pub client: Client,
}

impl S3BlobProvider {
    /// Creates a new [`S3BlobProvider`] from the provided url.
    pub fn new_http(url: reqwest::Url) -> Self {
        Self { base_url: url, client: reqwest::Client::new() }
    }
}

#[async_trait::async_trait]
impl BlobProvider for S3BlobProvider {
    #[allow(clippy::large_stack_frames)]
    async fn blob(
        &self,
        _block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        let url = format!("{}{}", self.base_url, hash);
        let response = self.client.get(&url).send().await.map_err(L1ProviderError::S3Provider)?;

        if response.status().is_success() {
            let blob_data = response.bytes().await.map_err(L1ProviderError::S3Provider)?;

            // Parse the blob data
            if blob_data.len() == BLOB_SIZE {
                let blob = Blob::try_from(blob_data.as_ref())
                    .map_err(|_| L1ProviderError::Other("Invalid blob data"))?;
                Ok(Some(Arc::new(blob)))
            } else {
                Ok(None)
            }
        } else if response.status() == 404 {
            Ok(None)
        } else {
            Err(L1ProviderError::Other("S3 client HTTP error"))
        }
    }
}
