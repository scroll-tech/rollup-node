//! Credit to <https://github.com/op-rs/kona/tree/main/crates/providers/providers-alloy>

use crate::{BlobProvider, L1ProviderError};
use alloy_eips::eip4844::{Blob, BlobTransactionSidecarItem};
use alloy_primitives::B256;
use alloy_rpc_types_beacon::sidecar::{BeaconBlobBundle, BlobData};
use lru::LruCache;
use reqwest::Client;
use std::{num::NonZeroUsize, sync::Arc};
use tokio::sync::Mutex;

/// The size of the blob cache for the provider.
const PROVIDER_BLOB_CACHE_SIZE: usize = 100;

/// An API response.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct APIResponse<T> {
    /// The data.
    pub data: T,
}

/// A reduced genesis data.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ReducedGenesisData {
    /// The genesis time.
    #[serde(rename = "genesis_time")]
    #[serde(with = "alloy_serde::quantity")]
    genesis_time: u64,
}

/// A reduced config data.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ReducedConfigData {
    /// The seconds per slot.
    #[serde(rename = "SECONDS_PER_SLOT")]
    #[serde(with = "alloy_serde::quantity")]
    seconds_per_slot: u64,
}

/// An implementation of blob provider using a beacon client.
#[derive(Debug, Clone)]
pub struct BeaconClientProvider {
    /// The base URL of the beacon API.
    pub base: String,
    /// The inner reqwest client.
    pub inner: Client,
    /// The cache for blobs from similar blocks.
    cache: Arc<Mutex<LruCache<B256, Arc<Blob>>>>,
    /// The genesis timestamp for the Beacon chain.
    pub genesis_timestamp: u64,
    /// The slot interval for the Beacon chain.
    pub slot_interval: u64,
}

impl BeaconClientProvider {
    /// The config spec engine api method.
    const SPEC_METHOD: &'static str = "eth/v1/config/spec";

    /// The beacon genesis engine api method.
    const GENESIS_METHOD: &'static str = "eth/v1/beacon/genesis";

    /// The blob sidecars engine api method prefix.
    const SIDECARS_METHOD_PREFIX: &'static str = "eth/v1/beacon/blob_sidecars";

    /// Creates a new [`BeaconClientProvider`] from the provided base url.
    pub async fn new_http(base: reqwest::Url) -> Self {
        // If base ends with a slash, remove it
        let mut base = base.to_string();
        if base.ends_with('/') {
            base.remove(base.len() - 1);
        }

        let cache = Arc::new(Mutex::new(LruCache::new(
            NonZeroUsize::new(PROVIDER_BLOB_CACHE_SIZE).expect("cache requires non-zero capacity"),
        )));
        let client = Client::new();

        let config = Self::config_spec(&base, &client)
            .await
            .expect("failed to fetch Beacon chain configuration");
        let genesis = Self::beacon_genesis(&base, &client)
            .await
            .expect("failed to fetch Beacon chain genesis info");

        Self {
            base,
            inner: Client::new(),
            cache,
            slot_interval: config.data.seconds_per_slot,
            genesis_timestamp: genesis.data.genesis_time,
        }
    }

    /// Returns the reduced configuration data for the Beacon client.
    async fn config_spec(
        base: &str,
        client: &Client,
    ) -> Result<APIResponse<ReducedConfigData>, reqwest::Error> {
        let first = client.get(format!("{}/{}", base, Self::SPEC_METHOD)).send().await?;
        first.json::<APIResponse<ReducedConfigData>>().await
    }

    /// Returns the Beacon genesis information.
    async fn beacon_genesis(
        base: &str,
        client: &Client,
    ) -> Result<APIResponse<ReducedGenesisData>, reqwest::Error> {
        let first = client.get(format!("{}/{}", base, Self::GENESIS_METHOD)).send().await?;
        first.json::<APIResponse<ReducedGenesisData>>().await
    }

    /// Returns the blobs for the provided slot.
    async fn blobs(&self, slot: u64) -> Result<Vec<BlobData>, reqwest::Error> {
        let url = format!("{}/{}/{}", self.base, Self::SIDECARS_METHOD_PREFIX, slot);
        let response = self.inner.get(&url).send().await?.error_for_status()?;
        let blob_bundle = response.json::<BeaconBlobBundle>().await?;
        Ok(blob_bundle.data)
    }

    /// Returns the beacon slot given a block timestamp.
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
impl BlobProvider for BeaconClientProvider {
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
