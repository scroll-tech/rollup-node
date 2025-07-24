//! Exposes the [`BlobProvider`] trait allowing to retrieve blobs.

mod anvil;
pub use anvil::AnvilBlobProvider;

mod client;
pub use client::BeaconClientProvider;

mod mock;
pub use mock::MockBeaconProvider;

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
            // TODO: implement AWS S3 blob provider.
            Self::Mock | Self::S3 => Arc::new(MockBeaconProvider::default()),
            Self::Anvil => Arc::new(AnvilBlobProvider::new_http(
                url.ok_or_eyre("missing url for anvil blob provider")?,
            )),
        })
    }
}
