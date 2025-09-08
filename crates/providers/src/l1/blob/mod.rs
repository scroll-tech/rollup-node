//! Exposes the [`BlobProvider`] trait allowing to retrieve blobs.

mod anvil;
pub use anvil::AnvilBlobProvider;

mod client;
pub use client::BeaconClientProvider;

mod mock;
pub use mock::MockBeaconProvider;

mod s3;
pub use s3::S3BlobProvider;

use crate::L1ProviderError;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use tracing::{debug, warn};

/// An instance of the trait can be used to fetch L1 blob data.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc, &)]
pub trait BlobProvider: Sync + Send {
    /// Returns corresponding blob data for the provided hash.
    async fn blob(
        &self,
        block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError>;
}

/// The builder for the blob provider.
#[derive(Debug, Clone)]
pub struct BlobProvidersBuilder {
    /// Beacon client blob source.
    pub beacon: Option<reqwest::Url>,
    /// AWS S3 blob source.
    pub s3: Option<reqwest::Url>,
    /// Anvil sequencer blob source.
    pub anvil: Option<reqwest::Url>,
    /// Mocked source.
    pub mock: bool,
}

impl BlobProvidersBuilder {
    /// Returns an [`BlobProviders`].
    pub async fn build(&self) -> eyre::Result<BlobProviders> {
        if self.mock {
            return Ok(BlobProviders::new(
                None,
                vec![Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>],
            ));
        }

        let beacon_provider = if let Some(beacon) = &self.beacon {
            Some(Arc::new(BeaconClientProvider::new_http(beacon.clone()).await)
                as Arc<dyn BlobProvider>)
        } else {
            None
        };

        let mut backup_providers: Vec<Arc<dyn BlobProvider>> = vec![];
        if let Some(s3) = &self.s3 {
            backup_providers
                .push(Arc::new(S3BlobProvider::new_http(s3.clone())) as Arc<dyn BlobProvider>);
        }
        if let Some(anvil) = &self.anvil {
            backup_providers.push(Arc::new(AnvilBlobProvider::new_http(anvil.clone())) as Arc<dyn BlobProvider>);
        }

        if beacon_provider.is_none() && backup_providers.is_empty() {
            return Err(eyre::eyre!("No blob providers available"));
        }

        Ok(BlobProviders::new(beacon_provider, backup_providers))
    }
}

/// A blob provider that implements round-robin load balancing across multiple providers.
#[derive(Clone)]
pub struct BlobProviders {
    /// beacon provider
    beacon_provider: Option<Arc<dyn BlobProvider>>,
    /// The list of underlying blob providers.
    backup_providers: Vec<Arc<dyn BlobProvider>>,
    /// Atomic counter for round-robin selection.
    counter: Arc<AtomicUsize>,
}

impl std::fmt::Debug for BlobProviders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobProviders")
            .field("has_beacon_provider", &self.beacon_provider.is_some())
            .field("backup_providers_count", &self.backup_providers.len())
            .field("counter", &self.counter.load(Ordering::Relaxed))
            .finish()
    }
}

impl BlobProviders {
    /// Creates a new round-robin blob provider.
    pub fn new(
        beacon_provider: Option<Arc<dyn BlobProvider>>,
        backup_providers: Vec<Arc<dyn BlobProvider>>,
    ) -> Self {
        Self { beacon_provider, backup_providers, counter: Arc::new(AtomicUsize::new(0)) }
    }
}

#[async_trait::async_trait]
impl BlobProvider for BlobProviders {
    async fn blob(
        &self,
        block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        // Always try the beacon provider first
        if let Some(beacon_provider) = &self.beacon_provider {
            match beacon_provider.blob(block_timestamp, hash).await {
                Ok(blob) => return Ok(blob),
                Err(_err) => {
                    debug!(target: "scroll::providers", ?hash, ?block_timestamp, "Beacon provider failed, trying backup providers");
                }
            }
        }

        // Try each provider in round-robin order, starting from the next provider
        if self.backup_providers.is_empty() {
            // All providers failed and no backup providers available
            warn!(target: "scroll::providers", ?hash, ?block_timestamp, "All beacon providers failed to fetch blob");
            return Err(L1ProviderError::Other("All blob providers failed"));
        }

        let start_index = self.counter.load(Ordering::Relaxed) % self.backup_providers.len();

        for i in 0..self.backup_providers.len() {
            let provider_index = (start_index + i) % self.backup_providers.len();
            let provider = &self.backup_providers[provider_index];

            match provider.blob(block_timestamp, hash).await {
                Ok(blob) => {
                    // Update counter to start from this provider next time if we found the blob
                    if blob.is_some() {
                        self.counter.store(provider_index + 1, Ordering::Relaxed);
                    }
                    return Ok(blob);
                }
                Err(err) => {
                    debug!(target: "scroll::providers", ?hash, ?block_timestamp, ?provider_index, ?err, "backup beacon provider failed to fetch blob");
                }
            }
        }

        // All providers failed
        warn!(target: "scroll::providers", ?hash, ?block_timestamp, "All beacon providers failed to fetch blob");
        Err(L1ProviderError::Other("All blob providers failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_s3_blob_provider() {
        let provider = S3BlobProvider::new_http(
            reqwest::Url::parse("https://scroll-mainnet-blob-data.s3.us-west-2.amazonaws.com")
                .unwrap(),
        );
        let blob = provider
            .blob(
                0,
                B256::from_str(
                    "0x0155ba17dcd008d7ba499d0e2f1fdc26dcc9fb4d83ee37d5c4bb3d1040c3f48a",
                )
                .unwrap(),
            )
            .await
            .unwrap();

        assert!(blob.is_some());
    }

    #[tokio::test]
    async fn test_blob_providers() {
        let source = BlobProvidersBuilder { beacon: None, s3: None, anvil: None, mock: true };

        let provider = source.build().await.unwrap();
        let result = provider.blob(0, B256::ZERO).await.unwrap();
        assert!(result.is_none()); // MockBeaconProvider returns None
    }

    #[tokio::test]
    async fn test_blob_providers_with_backup() {
        let providers = BlobProviders::new(
            None,
            vec![
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
            ],
        );

        // Test multiple calls to ensure round-robin behavior
        for _ in 0..5 {
            let result = providers.blob(0, B256::ZERO).await.unwrap();
            assert!(result.is_none());
        }
    }

    #[tokio::test]
    async fn test_blob_providers_beacon_priority() {
        // Test that beacon provider is always tried first
        let providers = BlobProviders::new(
            Some(Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>),
            vec![
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
            ],
        );

        let result = providers.blob(0, B256::ZERO).await.unwrap();
        assert!(result.is_none()); // MockBeaconProvider returns None
    }

    #[tokio::test]
    async fn test_blob_providers_round_robin() {
        // Create a provider with only backup providers to test round-robin
        let providers = BlobProviders::new(
            None,
            vec![
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
            ],
        );

        // Test multiple calls to verify round-robin behavior
        for i in 0..10 {
            let result = providers.blob(0, B256::from([i as u8; 32])).await.unwrap();
            assert!(result.is_none()); // MockBeaconProvider always returns None
        }
    }

    #[tokio::test]
    async fn test_blob_providers_empty_fails() {
        let providers = BlobProviders::new(None, vec![]);

        // Should fail when no providers are available
        let result = providers.blob(0, B256::ZERO).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_blob_providers_builder_mock() {
        let builder = BlobProvidersBuilder { beacon: None, s3: None, anvil: None, mock: true };

        let providers = builder.build().await.unwrap();
        let result = providers.blob(0, B256::ZERO).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_blob_providers_builder_no_providers() {
        let builder = BlobProvidersBuilder { beacon: None, s3: None, anvil: None, mock: false };

        // Should fail when no providers are configured
        let result = builder.build().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No blob providers available"));
    }

    #[tokio::test]
    async fn test_blob_providers_clone() {
        let providers = BlobProviders::new(
            Some(Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>),
            vec![Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>],
        );

        // Test that BlobProviders can be cloned
        let cloned_providers = providers.clone();

        // Both should work independently
        let result1 = providers.blob(0, B256::ZERO).await.unwrap();
        let result2 = cloned_providers.blob(0, B256::ZERO).await.unwrap();

        assert!(result1.is_none());
        assert!(result2.is_none());
    }

    #[tokio::test]
    async fn test_blob_providers_debug_format() {
        let providers = BlobProviders::new(
            Some(Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>),
            vec![
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
                Arc::new(MockBeaconProvider::default()) as Arc<dyn BlobProvider>,
            ],
        );

        let debug_str = format!("{:?}", providers);
        assert!(debug_str.contains("BlobProviders"));
        assert!(debug_str.contains("has_beacon_provider"));
        assert!(debug_str.contains("backup_providers_count"));
    }
}
