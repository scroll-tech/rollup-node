use alloy_primitives::Address;
use scroll_db::{L1MessageKey, NotIncludedStart};
use std::{fmt, str::FromStr, sync::Arc};

/// Configuration for the sequencer.
#[derive(Debug)]
pub struct SequencerConfig<CS> {
    /// The chain spec.
    pub chain_spec: Arc<CS>,
    /// The fee recipient.
    pub fee_recipient: Address,
    /// Whether the sequencer should start automatically.
    pub auto_start: bool,
    /// The payload building config.
    pub payload_building_config: PayloadBuildingConfig,
    /// The block time in milliseconds.
    pub block_time: u64,
    /// The duration in seconds to build payload attributes.
    pub payload_building_duration: u64,
    /// Whether to allow empty blocks.
    pub allow_empty_blocks: bool,
}

/// Configuration for building payloads.
#[derive(Debug, Clone)]
pub struct PayloadBuildingConfig {
    /// The block gas limit.
    pub block_gas_limit: u64,
    /// The number of L1 messages to include in each block.
    pub max_l1_messages_per_block: u64,
    /// The L1 message inclusion mode configuration.
    pub l1_message_inclusion_mode: L1MessageInclusionMode,
}

/// Configuration for L1 message inclusion strategy.
#[derive(Debug, Clone, Copy)]
pub enum L1MessageInclusionMode {
    /// Include L1 messages based on block depth.
    BlockDepth(u64),
    /// Include only finalized L1 messages with an additional block depth.
    FinalizedWithBlockDepth(u64),
}

// The default is to include finalized L1 messages with a depth of 2 blocks below the current
// finalized block number.
impl Default for L1MessageInclusionMode {
    fn default() -> Self {
        Self::FinalizedWithBlockDepth(2)
    }
}

impl FromStr for L1MessageInclusionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = s.strip_prefix("finalized:") {
            rest.parse::<u64>()
                .map(Self::FinalizedWithBlockDepth)
                .map_err(|_| format!("Expected a valid number after 'finalized:', got '{rest}'"))
        } else if s.eq_ignore_ascii_case("finalized") {
            Ok(Self::FinalizedWithBlockDepth(0))
        } else if let Some(rest) = s.strip_prefix("depth:") {
            rest.parse::<u64>()
                .map(Self::BlockDepth)
                .map_err(|_| format!("Expected a valid number after 'depth:', got '{rest}'"))
        } else {
            Err("Expected 'finalized' or 'depth:{number}' (e.g. 'depth:10')".to_string())
        }
    }
}

impl fmt::Display for L1MessageInclusionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FinalizedWithBlockDepth(depth) => write!(f, "finalized:{depth}"),
            Self::BlockDepth(depth) => write!(f, "depth:{depth}"),
        }
    }
}

impl From<L1MessageInclusionMode> for L1MessageKey {
    fn from(mode: L1MessageInclusionMode) -> Self {
        match mode {
            L1MessageInclusionMode::FinalizedWithBlockDepth(depth) => {
                Self::NotIncluded(NotIncludedStart::FinalizedWithBlockDepth(depth))
            }
            L1MessageInclusionMode::BlockDepth(depth) => {
                Self::NotIncluded(NotIncludedStart::BlockDepth(depth))
            }
        }
    }
}
