use moka::future::Cache;
use scroll_alloy_rpc_types_engine::BlockDataHint;
use scroll_db::{Database, DatabaseReadOperations};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

use crate::DerivationPipelineError;

/// The default size of the block data hint pre-fetch cache (number of entries).
pub(crate) const CACHE_SIZE: usize = 100_000;

/// The default number of block data hints to pre-fetch.
pub(crate) const PREFETCH_RANGE_SIZE: usize = 50_000;

/// The default time-to-live (TTL) for cache entries.
pub(crate) const CACHE_TTL: Duration = Duration::from_secs(120 * 60); // 120 minutes

#[derive(Debug, Clone)]
pub struct PreFetchCache {
    db: Arc<Database>,
    hint_cache: Cache<u64, BlockDataHint>,
    // TODO: Add a cache for batches.
    max_block_data_hint_block_number: u64,
    pre_fetch_count: usize,
    ranges_in_cache: Arc<Mutex<Vec<u64>>>,
}

impl PreFetchCache {
    /// Creates a new block data hint pre-fetch cache with default settings.
    pub(crate) async fn new(
        db: Arc<Database>,
        size: usize,
        ttl: Duration,
        pre_fetch_count: usize,
    ) -> Result<Self, DerivationPipelineError> {
        let max_block_data_hint_block_number = db.get_max_block_data_hint_block_number().await?;
        Ok(Self {
            db,
            hint_cache: Cache::builder().max_capacity(size as u64).time_to_live(ttl).build(),
            max_block_data_hint_block_number,
            pre_fetch_count,
            ranges_in_cache: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Fetches the block data hint for the given block number, using the cache if possible.
    pub(crate) async fn get(
        &self,
        block_number: u64,
    ) -> Result<Option<BlockDataHint>, DerivationPipelineError> {
        tracing::trace!(
            target: "scroll::derivation_pipeline::cache",
            block_number = block_number,
            "Fetching block data hint from cache",
        );
        // If the block number is beyond the maximum known block data hint, return None.
        if block_number > self.max_block_data_hint_block_number {
            return Ok(None);
        }

        // If the hint is already cached, return it.
        if let Some(cached_hint) = self.hint_cache.get(&block_number).await {
            return Ok(Some(cached_hint));
        }

        // The data hint is not in cache so we will pre-fetch a range of hints.
        let range_start = self.pre_fetch_range_start(block_number);
        let mut ranges_in_cache = self.ranges_in_cache.lock().await;

        // The range has already been fetched by another task (this is a rare case caused by a race)
        if !ranges_in_cache.contains(&range_start) {
            tracing::info!(
                target: "scroll::derivation_pipeline::cache",
                range_start = range_start,
                pre_fetch_count = self.pre_fetch_count,
                "Pre-fetching block data hints for range",
            );
            let hints = self.db.get_n_l2_block_data_hint(range_start, self.pre_fetch_count).await?;
            for (idx, hint) in hints.iter().enumerate() {
                self.hint_cache.insert(range_start + idx as u64, hint.clone()).await;
            }

            if ranges_in_cache.len() == 2 {
                ranges_in_cache.remove(0);
            }

            ranges_in_cache.push(range_start);
        };

        drop(ranges_in_cache);

        // Now the requested hint should be in cache.
        Ok(self.hint_cache.get(&block_number).await)
    }

    const fn pre_fetch_range_start(&self, block: u64) -> u64 {
        (block / self.pre_fetch_count as u64) * self.pre_fetch_count as u64
    }
}
