use crate::BlockInfo;
use alloy_chains::NamedChain;
use alloy_rpc_types_engine::ForkchoiceState as AlloyForkchoiceState;
use reth_scroll_chainspec::{SCROLL_MAINNET_GENESIS_HASH, SCROLL_SEPOLIA_GENESIS_HASH};

/// The fork choice state.
///
/// The state is composed of the [`BlockInfo`] for `unsafe`, `safe` block, and the `finalized`
/// blocks.
#[derive(Debug, Clone)]
pub struct ForkchoiceState {
    unsafe_: BlockInfo,
    safe: BlockInfo,
    finalized: BlockInfo,
}

impl ForkchoiceState {
    /// Creates a new [`ForkchoiceState`] instance from the given [`BlockInfo`] instance.
    pub const fn from_block_info(block_info: BlockInfo) -> Self {
        Self::new(block_info, block_info, block_info)
    }

    /// Creates a new [`ForkchoiceState`] instance.
    pub const fn new(unsafe_: BlockInfo, safe: BlockInfo, finalized: BlockInfo) -> Self {
        Self { unsafe_, safe, finalized }
    }

    /// Creates a [`ForkchoiceState`] instance that represents the genesis state of the provided
    /// chain.
    pub const fn genesis(chain: NamedChain) -> Self {
        let block_info = match chain {
            NamedChain::Scroll => BlockInfo { hash: SCROLL_MAINNET_GENESIS_HASH, number: 0 },
            NamedChain::ScrollSepolia => BlockInfo { hash: SCROLL_SEPOLIA_GENESIS_HASH, number: 0 },
            _ => panic!("unsupported chain"),
        };
        Self::new(block_info, block_info, block_info)
    }

    /// Updates the `unsafe` block info.
    pub fn update_unsafe_block_info(&mut self, unsafe_: BlockInfo) {
        self.unsafe_ = unsafe_;
    }

    /// Updates the `safe` block info.
    pub fn update_safe_block_info(&mut self, safe: BlockInfo) {
        self.safe = safe;
    }

    /// Updates the `finalized` block info.
    pub fn update_finalized_block_info(&mut self, finalized: BlockInfo) {
        self.finalized = finalized;
    }

    /// Returns the block info for the `unsafe` block.
    pub const fn unsafe_block_info(&self) -> &BlockInfo {
        &self.unsafe_
    }

    /// Returns the block info for the `safe` block.
    pub const fn safe_block_info(&self) -> &BlockInfo {
        &self.safe
    }

    /// Returns the block info for the `finalized` block.
    pub const fn finalized_block_info(&self) -> &BlockInfo {
        &self.finalized
    }

    /// Returns the [`AlloyForkchoiceState`] representation of the fork choice state.
    pub const fn get_alloy_fcs(&self) -> AlloyForkchoiceState {
        AlloyForkchoiceState {
            head_block_hash: self.unsafe_.hash,
            safe_block_hash: self.safe.hash,
            finalized_block_hash: self.finalized.hash,
        }
    }

    /// Returns `true` if the fork choice state is the genesis state.
    pub const fn is_genesis(&self) -> bool {
        self.unsafe_.number == 0
    }
}
