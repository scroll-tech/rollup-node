use crate::BlockInfo;

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
    /// Creates a new [`ForkchoiceState`] instance.
    pub fn new(unsafe_: BlockInfo, safe: BlockInfo, finalized: BlockInfo) -> Self {
        Self { unsafe_, safe, finalized }
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
    pub fn unsafe_block_info(&self) -> &BlockInfo {
        &self.unsafe_
    }

    /// Returns the block info for the `safe` block.
    pub fn safe_block_info(&self) -> &BlockInfo {
        &self.safe
    }

    /// Returns the block info for the `finalized` block.
    pub fn finalized_block_info(&self) -> &BlockInfo {
        &self.finalized
    }
}
