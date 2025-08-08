use crate::{BlobProvider, L1ProviderError};
use std::sync::Arc;

use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use alloy_rpc_client::RpcClient;

/// An implementation of a blob provider client using S3.
#[derive(Debug, Clone)]
pub struct S3BlobProvider {
    /// The base URL for the S3 service.
    pub base_url: reqwest::Url,
    /// HTTP client for making requests.
    pub client: reqwest::Client,
}

impl S3BlobProvider {
    /// Creates a new [`S3BlobProvider`] from the provided url.
    pub fn new_http(url: reqwest::Url) -> Self {
        Self { 
            base_url: url,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl BlobProvider for S3BlobProvider {
    async fn blob(
        &self,
        _block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        let url = format!("{}/{}", self.base_url, hash);
        let response = self.client.get(&url).send().await
            .map_err(|e| L1ProviderError::Transport(e.into()))?;
        
        if response.status().is_success() {
            let blob_data = response.bytes().await
                .map_err(|e| L1ProviderError::Transport(e.into()))?;
            
            // Parse the blob data - this depends on the S3 storage format
            // For now, assume it's stored as raw blob bytes
            if blob_data.len() == 131072 { // Standard blob size
                let blob = Blob::try_from(blob_data.as_ref())
                    .map_err(|e| L1ProviderError::Transport(e.into()))?;
                Ok(Some(Arc::new(blob)))
            } else {
                Ok(None)
            }
        } else if response.status() == 404 {
            Ok(None)
        } else {
            Err(L1ProviderError::Transport(
                format!("HTTP error: {}", response.status()).into()
            ))
        }
    }
}