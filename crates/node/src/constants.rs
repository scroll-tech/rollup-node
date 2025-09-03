//! Constants related to the [`crate::ScrollRollupNode`]

use alloy_primitives::U128;

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
pub(crate) const BLOCK_GAP_TRIGGER: u64 = 100_000;

/// The number of block headers to keep in the in-memory chain buffer in the chain orchestrator.
pub(crate) const CHAIN_BUFFER_SIZE: usize = 2000;

/// The default suggested priority fee for the gas price oracle.
pub(crate) const DEFAULT_SUGGEST_PRIORITY_FEE: u64 = 100;

/// Scroll default gas limit.
/// Should match <https://github.com/scroll-tech/reth/blob/scroll/crates/scroll/node/src/builder/payload.rs#L36>.
pub const SCROLL_GAS_LIMIT: u64 = 20_000_000;

/// The constant value that must be added to the block number to get the total difficulty for Scroll
/// mainnet.
pub(crate) const SCROLL_MAINNET_TD_CONSTANT: U128 = U128::from_limbs([14906960, 0]);

/// The constant value that must be added to the block number to get the total difficulty for Scroll
/// Sepolia.
pub(crate) const SCROLL_SEPOLIA_TD_CONSTANT: U128 = U128::from_limbs([8484488, 0]);

/// The L1 message queue index at which the V2 L1 message queue was enabled on mainnet.
pub(crate) const SCROLL_MAINNET_V2_MESSAGE_QUEUE_START_INDEX: u64 = 953885;

/// The L1 message queue index at which queue hashes should be computed on sepolia.
pub(crate) const SCROLL_SEPOLIA_V2_MESSAGE_QUEUE_START_INDEX: u64 = 1062110;
