pub(crate) mod blob;
pub(crate) mod message;

use crate::{
    beacon_client::OnlineBeaconClient,
    l1::message::{
        DatabaseL1MessageDelayProvider, L1MessageProvider, L1MessageWithBlockNumberProvider,
    },
    L1BlobProvider,
};
use std::{num::NonZeroUsize, sync::Arc};

use alloy_eips::eip4844::{Blob, BlobTransactionSidecarItem};
use alloy_primitives::B256;
use lru::LruCache;
use rollup_node_primitives::L1MessageWithBlockNumber;
use scroll_db::DatabaseError;
use tokio::sync::Mutex;

/// An instance of the trait can be used to provide L1 data.
pub trait L1Provider:
    L1BlobProvider + L1MessageWithBlockNumberProvider + L1MessageProvider
{
}
impl<T> L1Provider for T where
    T: L1BlobProvider + L1MessageWithBlockNumberProvider + L1MessageProvider
{
}

/// An error occurring at the [`L1Provider`].
#[derive(Debug, thiserror::Error)]
pub enum L1ProviderError {
    /// Invalid timestamp for slot.
    #[error("Beacon client error: {0}")]
    BeaconClient(#[from] reqwest::Error),
    /// Invalid timestamp for slot.
    #[error("invalid block timestamp: genesis {0}, provided {1}")]
    InvalidBlockTimestamp(u64, u64),
    /// Database error.
    #[error(transparent)]
    Database(#[from] DatabaseError),
    /// Other error.
    #[error("{0}")]
    Other(&'static str),
}

/// An online implementation of the [`L1Provider`] trait.
#[derive(Debug)]
pub struct OnlineL1Provider<P> {
    /// The Beacon client.
    beacon_client: OnlineBeaconClient,
    /// The cache for blobs from similar blocks.
    cache: Arc<Mutex<LruCache<B256, Arc<Blob>>>>,
    /// The L1 message provider
    l1_message_provider: P,
    /// The genesis timestamp for the Beacon chain.
    genesis_timestamp: u64,
    /// The slot interval for the Beacon chain.
    slot_interval: u64,
}

impl<P> OnlineL1Provider<P> {
    /// Returns a new [`OnlineBeaconClient`] from the provided [`OnlineBeaconClient`], blob capacity
    /// and [`L1MessageProvider`].
    pub async fn new(
        client: OnlineBeaconClient,
        blob_capacity: usize,
        l1_message_provider: P,
    ) -> Self {
        let cache = Arc::new(Mutex::new(LruCache::new(
            NonZeroUsize::new(blob_capacity).expect("cache requires non-zero capacity"),
        )));
        let config =
            client.config_spec().await.expect("failed to fetch Beacon chain configuration");
        let genesis =
            client.beacon_genesis().await.expect("failed to fetch Beacon chain genesis info");

        Self {
            beacon_client: client,
            cache,
            l1_message_provider,
            genesis_timestamp: genesis.data.genesis_time,
            slot_interval: config.data.seconds_per_slot,
        }
    }

    const fn slot(&self, block_timestamp: u64) -> Result<u64, L1ProviderError> {
        if block_timestamp < self.genesis_timestamp {
            return Err(L1ProviderError::InvalidBlockTimestamp(
                self.genesis_timestamp,
                block_timestamp,
            ))
        }

        Ok((block_timestamp - self.genesis_timestamp) / self.slot_interval)
    }
}

#[async_trait::async_trait]
impl<P: Sync> L1BlobProvider for OnlineL1Provider<P> {
    /// Returns the requested blob corresponding to the passed hash.
    async fn blob(
        &self,
        block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        // check if the requested blob is in the cache.
        let mut cache = self.cache.lock().await;
        if let Some(blob) = cache.get(&hash) {
            return Ok(Some(blob.clone()));
        }
        // avoid holding the lock over the blob request.
        drop(cache);

        // query the blobs with the client, return target blob and store all others in cache.
        let slot = self.slot(block_timestamp)?;
        let mut blobs = self
            .beacon_client
            .blobs(slot)
            .await?
            .into_iter()
            .map(|blob| BlobTransactionSidecarItem {
                index: blob.index,
                blob: blob.blob,
                kzg_commitment: blob.kzg_commitment,
                kzg_proof: blob.kzg_proof,
            })
            .collect::<Vec<_>>();

        // if we find a blob, timestamp is valid.
        // cache the other blobs and return the matched blob.
        let maybe_blob = blobs.iter().position(|blob| blob.to_kzg_versioned_hash() == hash.0);
        if let Some(position) = maybe_blob {
            let blob = Arc::new(*blobs.remove(position).blob);
            let mut cache = self.cache.lock().await;
            for (hash, blob) in
                blobs.iter().map(|b| (b.to_kzg_versioned_hash().into(), Arc::new(*b.blob)))
            {
                cache.put(hash, blob);
            }
            return Ok(Some(blob))
        }

        Ok(None)
    }
}

#[async_trait::async_trait]
impl<P: L1MessageWithBlockNumberProvider + Sync + Send> L1MessageWithBlockNumberProvider
    for OnlineL1Provider<P>
{
    type Error = <P>::Error;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageWithBlockNumber>, Self::Error> {
        self.l1_message_provider.get_l1_message_with_block_number().await
    }

    fn set_index_cursor(&mut self, index: u64) {
        self.l1_message_provider.set_index_cursor(index)
    }

    fn set_hash_cursor(&mut self, hash: B256) {
        self.l1_message_provider.set_hash_cursor(hash)
    }

    fn increment_cursor(&mut self) {
        self.l1_message_provider.increment_cursor()
    }
}
