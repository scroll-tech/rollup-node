/// The size of the blob cache for the provider.
pub const PROVIDER_BLOB_CACHE_SIZE: usize = 100;

/// The max retries for the L1 provider.
pub const PROVIDER_MAX_RETRIES: u32 = 10;

/// The initial backoff for the L1 provider.
pub const PROVIDER_INITIAL_BACKOFF: u64 = 100;

/// The default provider compute units per second.
pub const PROVIDER_COMPUTE_UNITS_PER_SECOND: u64 = 50;

/// The default block time in milliseconds for the sequencer.
pub const DEFAULT_BLOCK_TIME: u64 = 2000;

/// The default payload building duration in milliseconds for the sequencer.
pub const DEFAULT_PAYLOAD_BUILDING_DURATION: u64 = 500;

/// The default max L1 messages per block for the sequencer.
pub const DEFAULT_MAX_L1_MESSAGES_PER_BLOCK: u64 = 4;
