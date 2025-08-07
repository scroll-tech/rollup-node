//! Constants related to the [`crate::ScrollRollupNode`]

/// The max retries for the L1 provider.
pub(crate) const PROVIDER_MAX_RETRIES: u32 = 10;

/// The initial backoff for the L1 provider.
pub(crate) const PROVIDER_INITIAL_BACKOFF: u64 = 100;

/// The default provider compute units per second.
pub(crate) const PROVIDER_COMPUTE_UNITS_PER_SECOND: u64 = 10000;

/// The default block time in milliseconds for the sequencer.
pub(crate) const DEFAULT_BLOCK_TIME: u64 = 1000;

/// The default payload building duration in milliseconds for the sequencer.
pub(crate) const DEFAULT_PAYLOAD_BUILDING_DURATION: u64 = 800;

/// The default payload size limit in bytes for the sequencer.
pub(crate) const DEFAULT_PAYLOAD_SIZE_LIMIT: u64 = 122_880;

/// The gap in blocks between the P2P and EN which triggers sync.
pub(crate) const BLOCK_GAP_TRIGGER: u64 = 500_000;

/// The default suggested priority fee for the gas price oracle.
pub(crate) const DEFAULT_SUGGEST_PRIORITY_FEE: u64 = 100;

/// Scroll default gas limit.
/// Should match <https://github.com/scroll-tech/reth/blob/scroll/crates/scroll/node/src/builder/payload.rs#L36>.
pub const SCROLL_GAS_LIMIT: u64 = 20_000_000;
