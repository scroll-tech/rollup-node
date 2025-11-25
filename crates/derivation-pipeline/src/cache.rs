use moka::future::Cache;
use scroll_alloy_rpc_types_engine::BlockDataHint;
use scroll_db::{Database, DatabaseReadOperations};
use std::{sync::Arc, time::Duration};

use crate::DerivationPipelineError;

/// The default size of the block data hint pre-fetch cache (number of entries).
pub(crate) const DEFAULT_CACHE_SIZE: usize = 80_000;

/// The default number of block data hints to pre-fetch.
pub(crate) const DEFAULT_PREFETCH_COUNT: usize = 60_000;

/// The default time-to-live (TTL) for cache entries.
pub(crate) const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(120 * 60); // 120 minutes

#[derive(Debug, Clone)]
pub struct PreFetchCache {
    db: Arc<Database>,
    hint_cache: Cache<u64, BlockDataHint>,
    // TODO: Add a cache for batches.
    max_block_data_hint_block_number: u64,
    pre_fetch_count: usize,
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
        })
    }

    /// Fetches the block data hint for the given block number, using the cache if possible.
    pub(crate) async fn get(
        &self,
        block_number: u64,
    ) -> Result<Option<BlockDataHint>, DerivationPipelineError> {
        if block_number > self.max_block_data_hint_block_number {
            return Ok(None);
        }

        if let Some(cached_hint) = self.hint_cache.get(&block_number).await {
            return Ok(Some(cached_hint));
        }

        let hints = self.db.get_n_l2_block_data_hint(block_number, self.pre_fetch_count).await?;
        let requested = hints.first().cloned();
        for (idx, hint) in hints.into_iter().enumerate() {
            self.hint_cache.insert(block_number + idx as u64, hint.clone()).await;
        }

        Ok(requested)
    }
}
