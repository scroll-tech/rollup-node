use crate::L1ProviderError;

use alloy_primitives::B256;
use futures::Stream;
use rollup_node_primitives::L1MessageEnvelope;
use scroll_db::{DatabaseError, DatabaseOperations, L1MessageStart};

/// An instance of the trait can provide L1 messages iterators.
#[async_trait::async_trait]
pub trait L1MessageProvider: Send + Sync {
    /// The error type for the provider.
    type Error: Into<L1ProviderError> + Send;

    /// Returns the next `n` L1 messages starting from the given index.
    /// Required in the sequencer because of <https://github.com/rust-lang/rust/issues/100013>,
    /// which seems to disallow using `iter_messages_from_index` and collecting `n` items from it.
    async fn take_n_messages_from_index(
        &self,
        start_index: u64,
        n: u64,
    ) -> Result<Vec<L1MessageEnvelope>, Self::Error>;

    /// Returns a stream of L1 messages starting from the given index.
    async fn iter_messages_from_index(
        &self,
        start_index: u64,
    ) -> Result<impl Stream<Item = Result<L1MessageEnvelope, Self::Error>> + Send, Self::Error>;

    /// Returns a stream of L1 messages starting from the given queue hash.
    async fn iter_messages_from_queue_hash(
        &self,
        queue_hash: B256,
    ) -> Result<impl Stream<Item = Result<L1MessageEnvelope, Self::Error>> + Send, Self::Error>;
}

#[async_trait::async_trait]
impl<T> L1MessageProvider for T
where
    T: DatabaseOperations + Send + Sync,
{
    type Error = DatabaseError;

    async fn take_n_messages_from_index(
        &self,
        start_index: u64,
        n: u64,
    ) -> Result<Vec<L1MessageEnvelope>, Self::Error> {
        self.get_n_l1_messages(Some(L1MessageStart::Index(start_index)), n).await
    }

    async fn iter_messages_from_index(
        &self,
        start_index: u64,
    ) -> Result<impl Stream<Item = Result<L1MessageEnvelope, Self::Error>> + Send, Self::Error>
    {
        self.get_l1_messages(Some(L1MessageStart::Index(start_index))).await
    }

    async fn iter_messages_from_queue_hash(
        &self,
        queue_hash: B256,
    ) -> Result<impl Stream<Item = Result<L1MessageEnvelope, Self::Error>> + Send, Self::Error>
    {
        self.get_l1_messages(Some(L1MessageStart::Hash(queue_hash))).await
    }
}
