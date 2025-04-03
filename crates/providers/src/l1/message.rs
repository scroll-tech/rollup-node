use crate::L1ProviderError;
use std::sync::atomic::{AtomicU64, Ordering};

use alloy_primitives::B256;
use scroll_alloy_consensus::TxL1Message;
use scroll_db::{DatabaseConnectionProvider, DatabaseOperations};

/// An instance of the trait can provide L1 messages using a cursor approach. Set the cursor for the
/// provider using the queue index or hash and then call [`L1MessageProvider::next_l1_message`] to
/// iterate the queue.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait L1MessageProvider {
    /// The error type for the provider.
    type Error: Into<L1ProviderError>;

    /// Returns the L1 message at the current cursor and advances the cursor.
    async fn next_l1_message(&self) -> Result<Option<TxL1Message>, Self::Error>;
    /// Set the index cursor for the provider.
    fn set_index_cursor(&self, index: u64);
    /// Set the hash cursor for the provider.
    fn set_hash_cursor(&self, hash: B256);
}

/// Implements [`L1MessageProvider`] via a database connection.
#[derive(Debug)]
pub struct DatabaseL1MessageProvider<DB> {
    /// A connection to the database.
    database_connection: DB,
    /// The current L1 message index.
    index: AtomicU64,
}

impl<DB> DatabaseL1MessageProvider<DB> {
    /// Returns a new instance of the [`DatabaseL1MessageProvider`].
    pub const fn new(db: DB, index: u64) -> Self {
        Self { database_connection: db, index: AtomicU64::new(index) }
    }
}

#[async_trait::async_trait]
impl<DB: DatabaseConnectionProvider + Sync> L1MessageProvider for DatabaseL1MessageProvider<DB> {
    type Error = L1ProviderError;

    async fn next_l1_message(&self) -> Result<Option<TxL1Message>, Self::Error> {
        Ok(self
            .database_connection
            .get_l1_message(self.index.load(Ordering::Relaxed))
            .await
            .map(|tx| tx.map(|tx| tx.transaction))?)
    }

    // TODO: check if ordering relaxed is sufficient.
    fn set_index_cursor(&self, index: u64) {
        self.index.store(index, Ordering::Relaxed);
    }

    fn set_hash_cursor(&self, _hash: B256) {
        // TODO: issue 43
        todo!()
    }
}
