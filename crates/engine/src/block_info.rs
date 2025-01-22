use alloy_primitives::B256;
use alloy_rpc_types_engine::ExecutionPayload;

/// Information about a block.
#[derive(Debug, Copy, Clone)]
pub struct BlockInfo {
    /// The block number.
    pub number: u64,
    /// The block hash.
    pub hash: B256,
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
