use super::*;
use scroll_db::DatabaseTransactionProvider;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::Mutex;

/// Implements [`L1MessageProvider`] via a database connection.
#[derive(Debug)]
pub struct DatabaseL1MessageProvider<DB> {
    /// A connection to the database.
    database_connection: DB,
    /// The current L1 message index.
    index: AtomicU64,
    /// The current queue hash.
    queue_hash: Arc<Mutex<Option<B256>>>,
}

impl<DB> DatabaseL1MessageProvider<DB> {
    /// Returns a new instance of the [`DatabaseL1MessageProvider`].
    pub fn new(db: DB, index: u64) -> Self {
        Self {
            database_connection: db,
            index: AtomicU64::new(index),
            queue_hash: Default::default(),
        }
    }
}

/// Cloning the [`DatabaseL1MessageProvider`] clones the reference to the database and creates a new
/// u64 atomic.
impl<DB: Clone> Clone for DatabaseL1MessageProvider<DB> {
    fn clone(&self) -> Self {
        Self {
            database_connection: self.database_connection.clone(),
            index: AtomicU64::new(0),
            queue_hash: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl<DB: DatabaseTransactionProvider + Send + Sync> L1MessageProvider
    for DatabaseL1MessageProvider<DB>
{
    type Error = L1ProviderError;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageEnvelope>, Self::Error> {
        if let Some(hash) = self.queue_hash.lock().await.take() {
            let tx = self.database_connection.tx().await?;
            let message = tx.get_l1_message_by_hash(hash).await?;
            if let Some(message) = &message {
                self.index.store(message.transaction.queue_index, Ordering::Relaxed);
            }
            Ok(message)
        } else {
            let index = self.index.load(Ordering::Relaxed);
            let tx = self.database_connection.tx().await?;
            Ok(tx.get_l1_message_by_index(index).await?)
        }
    }

    fn set_queue_index_cursor(&self, index: u64) {
        self.index.store(index, Ordering::Relaxed);
    }

    async fn set_hash_cursor(&self, hash: B256) {
        *self.queue_hash.lock().await = Some(hash);
    }

    fn increment_cursor(&self) {
        self.index.fetch_add(1, Ordering::Relaxed);
    }
}
