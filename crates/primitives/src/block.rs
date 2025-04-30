use alloy_eips::BlockNumHash;
use alloy_primitives::B256;
use alloy_rpc_types_engine::{ExecutionPayload, PayloadError};
use reth_primitives_traits::transaction::signed::SignedTransaction;
use reth_scroll_primitives::ScrollBlock;

/// Information about a block.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    /// The block number.
    pub number: u64,
    /// The block hash.
    pub hash: B256,
}

impl BlockInfo {
    /// Returns a new instance of [`BlockInfo`].
    pub const fn new(number: u64, hash: B256) -> Self {
        Self { number, hash }
    }
}

impl From<ExecutionPayload> for BlockInfo {
    fn from(value: ExecutionPayload) -> Self {
        (&value).into()
    }
}

impl From<&ExecutionPayload> for BlockInfo {
    fn from(value: &ExecutionPayload) -> Self {
        Self { number: value.block_number(), hash: value.block_hash() }
    }
}

impl From<BlockNumHash> for BlockInfo {
    fn from(value: BlockNumHash) -> Self {
        Self { number: value.number, hash: value.hash }
    }
}

impl From<&ScrollBlock> for BlockInfo {
    fn from(value: &ScrollBlock) -> Self {
        Self { number: value.number, hash: value.hash_slow() }
    }
}

/// This struct represents an L2 block with a vector the hashes of the L1 messages included in the
/// block.
#[derive(Debug, Clone)]
pub struct L2BlockInfoWithL1Messages {
    /// The block info.
    pub block_info: BlockInfo,
    /// The hashes of the L1 messages included in the block.
    pub l1_messages: Vec<B256>,
}

impl TryFrom<ExecutionPayload> for L2BlockInfoWithL1Messages {
    type Error = PayloadError;

    fn try_from(value: ExecutionPayload) -> Result<Self, Self::Error> {
        value.try_into_block().map(Into::into)
    }
}

impl From<ScrollBlock> for L2BlockInfoWithL1Messages {
    // TODO: It would be more efficient to convert payload transactions from bytes to
    // ScrollSignedTransaction directly, instead of converting the whole block (we would avoid
    // calculating the transactions state root)
    fn from(value: ScrollBlock) -> Self {
        let block_number = value.number;
        let block_hash = value.hash_slow();
        let l1_messages = value
            .body
            .transactions
            .into_iter()
            .filter_map(|tx| tx.is_l1_message().then(|| *tx.tx_hash()))
            .collect();
        Self { block_info: BlockInfo { number: block_number, hash: block_hash }, l1_messages }
    }
}
