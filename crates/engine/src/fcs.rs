use crate::FcsError;
use alloy_chains::NamedChain;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Sealable, B256};
use alloy_provider::Provider;
use alloy_rpc_types_engine::ForkchoiceState as AlloyForkchoiceState;
use reth_chainspec::EthChainSpec;
use reth_primitives_traits::BlockHeader;
use reth_scroll_chainspec::{SCROLL_MAINNET_GENESIS_HASH, SCROLL_SEPOLIA_GENESIS_HASH};
use rollup_node_primitives::BlockInfo;
use scroll_alloy_network::Scroll;

/// The fork choice state.
///
/// The state is composed of the [`BlockInfo`] for `head`, `safe` block, and the `finalized`
/// blocks.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

    /// Creates a new [`ForkchoiceState`] instance setting the `head`, `safe` and `finalized` block
    /// info to the provided `genesis` hash.
    pub const fn from_genesis(genesis: B256) -> Self {
        Self::new(
            BlockInfo { hash: genesis, number: 0 },
            BlockInfo { hash: genesis, number: 0 },
            BlockInfo { hash: genesis, number: 0 },
        )
    }

    /// Creates a [`ForkchoiceState`] instance setting the `head`, `safe` and `finalized` hash to
    /// the appropriate genesis values by reading from the provider.
    pub async fn from_provider<P: Provider<Scroll>>(provider: &P) -> Option<Self> {
        let latest_block =
            provider.get_block(BlockId::Number(BlockNumberOrTag::Latest)).await.ok()??;
        let safe_block =
            provider.get_block(BlockId::Number(BlockNumberOrTag::Safe)).await.ok()??;
        let finalized_block =
            provider.get_block(BlockId::Number(BlockNumberOrTag::Finalized)).await.ok()??;
        Some(Self {
            head: BlockInfo { number: latest_block.header.number, hash: latest_block.header.hash },
            safe: BlockInfo { number: safe_block.header.number, hash: safe_block.header.hash },
            finalized: BlockInfo {
                number: finalized_block.header.number,
                hash: finalized_block.header.hash,
            },
        })
    }

    /// Creates a [`ForkchoiceState`] instance setting the `head`, `safe` and `finalized` hash to
    /// the appropriate genesis values depending on the named chain.
    pub fn head_from_chain_spec<CS: EthChainSpec<Header: BlockHeader>>(
        chain_spec: CS,
    ) -> Option<Self> {
        Some(Self::from_genesis(genesis_hash_from_chain_spec(chain_spec)?))
    }

    /// Update the forkchoice state with the given `head`, `safe` and `finalized` block info.
    pub fn update(
        &mut self,
        head: Option<BlockInfo>,
        safe: Option<BlockInfo>,
        finalized: Option<BlockInfo>,
    ) -> Result<(), FcsError> {
        tracing::debug!(target: "scroll::engine::fcs", ?head, ?safe, ?finalized, current = ?self, "Updating fork choice state");
        // Check that at least one of head, safe or finalized is Some.
        if head.is_none() && safe.is_none() && finalized.is_none() {
            return Err(FcsError::NoUpdateProvided);
        }

        // Build the candidate new state.
        let new_finalized = finalized.unwrap_or(self.finalized);
        let new_safe = safe.unwrap_or(self.safe);
        let new_head = head.unwrap_or(self.head);

        // Check that the finalized block number is increasing or stays the same with the same hash.
        if new_finalized.number <= self.finalized.number && new_finalized != self.finalized {
            return Err(FcsError::FinalizedBlockNumberNotIncreasing);
        }

        // Assert invariants: head >= safe >= finalized.
        if new_head.number < new_safe.number {
            return Err(FcsError::HeadBelowSafe);
        }

        if new_safe.number < new_finalized.number {
            return Err(FcsError::SafeBelowFinalized);
        }

        // Commit the state.
        self.head = new_head;
        self.safe = new_safe;
        self.finalized = new_finalized;

        Ok(())
    }

    /// Updates the `head` block info.
    pub fn update_head_block_info(&mut self, head: BlockInfo) -> Result<(), FcsError> {
        self.update(Some(head), None, None)
    }

    /// Updates the `safe` block info.
    pub fn update_safe_block_info(&mut self, safe: BlockInfo) -> Result<(), FcsError> {
        self.update(None, Some(safe), None)
    }

    /// Updates the `finalized` block info.
    pub fn update_finalized_block_info(&mut self, finalized: BlockInfo) -> Result<(), FcsError> {
        self.update(None, None, Some(finalized))
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

    /// Returns the [`AlloyForkchoiceState`] representation of the fork choice state, with the safe
    /// and finalized hashes set to 0x0.
    pub fn get_alloy_optimistic_fcs(&self) -> AlloyForkchoiceState {
        AlloyForkchoiceState {
            head_block_hash: self.head.hash,
            safe_block_hash: B256::default(),
            finalized_block_hash: B256::default(),
        }
    }

    /// Returns `true` if the fork choice state is the genesis state.
    pub const fn is_genesis(&self) -> bool {
        self.head.number == 0
    }
}

/// Returns the genesis hash for the given chain spec.
pub fn genesis_hash_from_chain_spec<CS: EthChainSpec<Header: BlockHeader>>(
    chain_spec: CS,
) -> Option<B256> {
    match chain_spec.chain().named() {
        Some(NamedChain::Scroll) => Some(SCROLL_MAINNET_GENESIS_HASH),
        Some(NamedChain::ScrollSepolia) => Some(SCROLL_SEPOLIA_GENESIS_HASH),
        Some(NamedChain::Dev) | None => Some(chain_spec.genesis_header().hash_slow()),
        _ => None,
    }
}
