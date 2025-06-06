use crate::L1ProviderError;

use scroll_alloy_rpc_types_engine::BlockDataHint;
use scroll_db::{DatabaseConnectionProvider, DatabaseError, DatabaseOperations};

/// Trait implementers can return block data.
#[async_trait::async_trait]
pub trait BlockDataProvider {
    /// The error type for the provider.
    type Error: Into<L1ProviderError>;

    /// Returns the block data for the provided block number.
    async fn block_data(&self, block_number: u64) -> Result<Option<BlockDataHint>, Self::Error>;
}

#[async_trait::async_trait]
impl<T> BlockDataProvider for T
where
    T: DatabaseConnectionProvider + Sync,
{
    type Error = DatabaseError;

    async fn block_data(&self, block_number: u64) -> Result<Option<BlockDataHint>, Self::Error> {
        self.get_l2_block_data_hint(block_number).await
    }
}
