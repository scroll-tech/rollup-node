use alloy_primitives::B256;
use alloy_rpc_types_engine::ExecutionPayload;

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
