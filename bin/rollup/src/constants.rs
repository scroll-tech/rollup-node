/// The block number at which to start the L1 watcher.
pub const WATCHER_START_BLOCK_NUMBER: u64 = 18318215;

/// The size of the blob cache for the provider.
pub const PROVIDER_BLOB_CACHE_SIZE: usize = 100;

/// The max retries for the L1 provider.
pub const PROVIDER_MAX_RETRIES: u32 = 10;

/// The initial backoff for the L1 provider.
pub const PROVIDER_INITIAL_BACKOFF: u64 = 100;
