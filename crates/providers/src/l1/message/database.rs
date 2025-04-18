use super::*;
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
impl<DB: DatabaseConnectionProvider + Send + Sync> L1MessageProvider
    for DatabaseL1MessageProvider<DB>
{
    type Error = L1ProviderError;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageEnvelope>, Self::Error> {
        if let Some(hash) = self.queue_hash.lock().await.take() {
            let message = self.database_connection.get_l1_message_by_hash(hash).await?;
            if let Some(message) = &message {
                self.index.store(message.transaction.queue_index, Ordering::Relaxed);
            }
            Ok(message)
        } else {
            let index = self.index.load(Ordering::Relaxed);
            Ok(self.database_connection.get_l1_message_by_index(index).await?)
        }
    }

    fn set_index_cursor(&self, index: u64) {
        self.index.store(index, Ordering::Relaxed);
    }

    async fn set_hash_cursor(&self, hash: B256) {
        *self.queue_hash.lock().await = Some(hash);
    }

    fn increment_cursor(&self) {
        self.index.fetch_add(1, Ordering::Relaxed);
    }
}

/// A provider that can provide L1 messages with a delay.
/// This provider is used to delay the L1 messages by a certain number of blocks which builds
/// confidence in the L1 message not being reorged.
#[derive(Debug)]
pub struct DatabaseL1MessageDelayProvider<DB> {
    /// The database L1 message provider.
    l1_message_provider: DatabaseL1MessageProvider<DB>,
    /// The current L1 block number.
    l1_tip: AtomicU64,
    /// The number of blocks to wait for before including a L1 message in a block.
    l1_message_delay: u64,
}

impl<DB> DatabaseL1MessageDelayProvider<DB> {
    /// Returns a new instance of the [`DatabaseL1MessageDelayProvider`].
    pub fn new(
        l1_message_provider: DatabaseL1MessageProvider<DB>,
        l1_tip_block_number: u64,
        l1_message_delay: u64,
    ) -> Self {
        Self { l1_message_provider, l1_tip: l1_tip_block_number.into(), l1_message_delay }
    }

    /// Sets the block number of the current L1 head.
    pub fn set_l1_head(&self, block_number: u64) {
        self.l1_tip.store(block_number, Ordering::Relaxed);
    }
}

/// A trait that allows the L1 message delay provider to set the current head number.
pub trait L1MessageDelayProvider {
    /// Set the number of the current L1 head block number.
    fn set_l1_head(&self, _block_number: u64) {}
}

impl<DB> L1MessageDelayProvider for DatabaseL1MessageDelayProvider<DB> {
    fn set_l1_head(&self, block_number: u64) {
        Self::set_l1_head(self, block_number);
    }
}

impl<DB> L1MessageDelayProvider for DatabaseL1MessageProvider<DB> {}

#[async_trait::async_trait]
impl<DB: DatabaseConnectionProvider + Send + Sync> L1MessageProvider
    for DatabaseL1MessageDelayProvider<DB>
{
    type Error = L1ProviderError;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageEnvelope>, Self::Error> {
        let msg = self.l1_message_provider.get_l1_message_with_block_number().await?;
        let result = if let Some(msg) = msg {
            let tx_block_number = msg.block_number;
            let depth = self.l1_tip.load(Ordering::Relaxed) - tx_block_number;
            (depth >= self.l1_message_delay).then_some(msg)
        } else {
            None
        };

        Ok(result)
    }

    fn set_index_cursor(&self, index: u64) {
        self.l1_message_provider.set_index_cursor(index);
    }

    async fn set_hash_cursor(&self, hash: B256) {
        self.l1_message_provider.set_hash_cursor(hash).await;
    }

    fn increment_cursor(&self) {
        self.l1_message_provider.increment_cursor();
    }
}
