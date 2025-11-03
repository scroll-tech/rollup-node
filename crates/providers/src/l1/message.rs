use crate::L1ProviderError;

use rollup_node_primitives::L1MessageEnvelope;
use scroll_db::{DatabaseError, DatabaseReadOperations, L1MessageKey};

/// An instance of the trait can provide L1 messages iterators.
#[async_trait::async_trait]
pub trait L1MessageProvider: Send + Sync {
    /// The error type for the provider.
    type Error: Into<L1ProviderError> + Send;

    /// Returns the next `n` L1 messages starting from the given start point. The `Stream` solution
    /// using `get_l1_messages` would be more elegant, but captures the lifetime of `self`,
    /// which prevents us from implementing `L1MessageProvider` for `T: DatabaseTransactionProvider`
    /// (because we end up returning a `Stream` referencing a local variable). Another solution
    /// would be to implement `ReadConnectionProvider` for `Arc<Database>` but goes against the
    /// current pattern of using `Tx` or `TxMut` to access the database.
    ///
    /// Because we know the exact amount of messages we want to fetch in the sequencer or derivation
    /// pipeline, we prefer a solution which allows us to use `T: DatabaseTransactionProvider` and
    /// avoid capturing the lifetime of `self`.
    async fn get_n_messages(
        &self,
        start: L1MessageKey,
        n: u64,
    ) -> Result<Vec<L1MessageEnvelope>, Self::Error>;
}

#[async_trait::async_trait]
impl<T> L1MessageProvider for T
where
    T: DatabaseReadOperations + Send + Sync,
{
    type Error = DatabaseError;

    async fn get_n_messages(
        &self,
        start: L1MessageKey,
        n: u64,
    ) -> Result<Vec<L1MessageEnvelope>, Self::Error> {
        self.get_n_l1_messages(Some(start), n as usize).await
    }
}
