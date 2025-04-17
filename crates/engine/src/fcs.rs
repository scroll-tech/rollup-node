use alloy_chains::NamedChain;
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState as AlloyForkchoiceState;
use reth_scroll_chainspec::{SCROLL_MAINNET_GENESIS_HASH, SCROLL_SEPOLIA_GENESIS_HASH};
use rollup_node_primitives::BlockInfo;

/// The fork choice state.
///
/// The state is composed of the [`BlockInfo`] for `head`, `safe` block, and the `finalized`
/// blocks.
#[derive(Debug, Clone)]
pub struct ForkchoiceState {
    head: BlockInfo,
    safe: BlockInfo,
    finalized: BlockInfo,
}

impl ForkchoiceState {
    /// Creates a new [`ForkchoiceState`] instance from the given [`BlockInfo`] instance.
    pub const fn from_block_info(block_info: BlockInfo) -> Self {
        Self::new(block_info, block_info, block_info)
    }

    /// Creates a new [`ForkchoiceState`] instance.
    pub const fn new(head: BlockInfo, safe: BlockInfo, finalized: BlockInfo) -> Self {
        Self { head, safe, finalized }
    }

    /// Creates a new [`ForkchoiceState`] instance from the genesis block hash.
    pub fn from_genesis(genesis: B256) -> Self {
        Self::new(
            BlockInfo { hash: genesis, number: 0 },
            BlockInfo { hash: Default::default(), number: 0 },
            BlockInfo { hash: Default::default(), number: 0 },
        )
    }

    /// Creates a [`ForkchoiceState`] instance that represents the genesis state of the provided
    /// chain.
    pub fn from_named(chain: NamedChain) -> Self {
        let block_info = match chain {
            NamedChain::Scroll => BlockInfo { hash: SCROLL_MAINNET_GENESIS_HASH, number: 0 },
            NamedChain::ScrollSepolia => BlockInfo { hash: SCROLL_SEPOLIA_GENESIS_HASH, number: 0 },
            _ => panic!("unsupported chain"),
        };
        Self::new(block_info, Default::default(), Default::default())
    }

    /// Updates the `head` block info.
    pub fn update_head_block_info(&mut self, unsafe_: BlockInfo) {
        self.head = unsafe_;
    }

    /// Updates the `safe` block info.
    pub fn update_safe_block_info(&mut self, safe: BlockInfo) {
        self.safe = safe;
    }

    /// Updates the `finalized` block info.
    pub fn update_finalized_block_info(&mut self, finalized: BlockInfo) {
        self.finalized = finalized;
    }

    /// Returns the block info for the `head` block.
    pub const fn head_block_info(&self) -> &BlockInfo {
        &self.head
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
            head_block_hash: self.head.hash,
            safe_block_hash: self.safe.hash,
            finalized_block_hash: self.finalized.hash,
        }
    }

    /// Returns `true` if the fork choice state is the genesis state.
    pub const fn is_genesis(&self) -> bool {
        self.head.number == 0
    }
}
