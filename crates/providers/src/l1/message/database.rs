use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Cloning the [`DatabaseL1MessageProvider`] clones the reference to the database and creates a new
/// u64 atomic.
impl<DB: Clone> Clone for DatabaseL1MessageProvider<DB> {
    fn clone(&self) -> Self {
        Self { database_connection: self.database_connection.clone(), index: AtomicU64::new(0) }
    }
}

#[async_trait::async_trait]
impl<DB: DatabaseConnectionProvider + Sync> L1MessageProvider for DatabaseL1MessageProvider<DB> {
    type Error = L1ProviderError;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageWithBlockNumber>, Self::Error> {
        let index = self.index.load(Ordering::Relaxed);
        Ok(self.database_connection.get_l1_message(index).await?)
    }

    fn set_index_cursor(&self, index: u64) {
        self.index.store(index, Ordering::Relaxed);
    }

    fn set_hash_cursor(&self, _hash: B256) {
        // TODO: issue 43
        todo!()
    }

    fn increment_cursor(&self) {
        self.index.fetch_add(1, Ordering::Relaxed);
    }
}
