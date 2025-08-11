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
use std::sync::Arc;

use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use eyre::OptionExt;
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

/// The blob source for the beacon provider.
#[derive(Debug, clap::ValueEnum, Default, Clone)]
pub enum BlobSource {
    /// Beacon client blob source.
    #[default]
    Beacon,
    /// Mocked source.
    Mock,
    /// AWS S3 blob source.
    S3,
    /// Anvil sequencer blob source.
    Anvil,
}

impl BlobSource {
    /// Returns an [`Arc<dyn BlobProvider>`] for the provided URL.
    pub async fn provider(&self, url: Option<reqwest::Url>) -> eyre::Result<Arc<dyn BlobProvider>> {
        Ok(match self {
            Self::Beacon => Arc::new(
                BeaconClientProvider::new_http(
                    url.ok_or_eyre("missing url for consensus client provider")?,
                )
                .await,
            ),
            Self::Mock => Arc::new(MockBeaconProvider::default()),
            Self::S3 => Arc::new(S3BlobProvider::new_http(
                url.ok_or_eyre("missing url for s3 blob provider")?,
            )),
            Self::Anvil => Arc::new(AnvilBlobProvider::new_http(
                url.ok_or_eyre("missing url for anvil blob provider")?,
            )),
        })
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
}
