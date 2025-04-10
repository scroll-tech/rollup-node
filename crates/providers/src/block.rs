use crate::L1ProviderError;

use alloy_eips::BlockId;
use alloy_primitives::{Bytes, U256};
use scroll_db::{DatabaseConnectionProvider, DatabaseError, DatabaseOperations};

/// Trait implementors can return block data.
#[async_trait::async_trait]
pub trait BlockDataProvider {
    /// The error type for the provider.
    type Error: Into<L1ProviderError>;

    /// Returns the extra data for the provided [`BlockId`].
    async fn extra_data(&self, block_id: BlockId) -> Result<Option<Bytes>, Self::Error>;

    /// Returns the difficulty for the provided [`BlockId`].
    async fn difficulty(&self, block_id: BlockId) -> Result<Option<U256>, Self::Error>;
}

#[async_trait::async_trait]
impl<T> BlockDataProvider for T
where
    T: DatabaseConnectionProvider + Sync,
{
    type Error = DatabaseError;

    async fn extra_data(&self, block_id: BlockId) -> Result<Option<Bytes>, Self::Error> {
        self.get_block_data(block_id).await.map(|data| data.map(|data| data.extra_data))
    }

    async fn difficulty(&self, block_id: BlockId) -> Result<Option<U256>, Self::Error> {
        self.get_block_data(block_id).await.map(|data| data.map(|data| data.difficulty))
    }
}
