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

/// A provider that can provide L1 messages with a delay.
/// This provider is used to delay the L1 messages by a certain number of blocks which builds
/// confidence in the L1 message not being reorged.
#[derive(Debug)]
pub struct DatabaseL1MessageDelayProvider<DB> {
    /// The database L1 message provider.
    l1_message_provider: DatabaseL1MessageProvider<DB>,
    /// The current L1 block number.
    l1_head: u64,
    /// The number of blocks to wait for before including a L1 message in a block.
    l1_message_delay: u64,
}

impl<DB> DatabaseL1MessageDelayProvider<DB> {
    /// Returns a new instance of the [`DatabaseL1MessageDelayProvider`].
    pub fn new(
        l1_message_provider: DatabaseL1MessageProvider<DB>,
        current_head_number: u64,
        l1_message_delay: u64,
    ) -> Self {
        Self { l1_message_provider, l1_head: current_head_number, l1_message_delay }
    }

    /// Sets the block number of the current L1 head.
    pub fn set_l1_head(&mut self, current_head_number: u64) {
        self.l1_head = current_head_number;
    }
}

/// A trait that allows the L1 message delay provider to set the current head number.
pub trait L1MessageDelayProvider {
    /// Set the number of the current L1 head.
    fn set_l1_head(&mut self, l1_head: u64);
}

impl<DB> L1MessageDelayProvider for DatabaseL1MessageDelayProvider<DB> {
    fn set_l1_head(&mut self, current_head_number: u64) {
        self.set_l1_head(current_head_number);
    }
}

/// A delay predicate that checks if the L1 message is delayed by a certain number of blocks.
fn validate_delay_predicate(
    msg_w_bn: &L1MessageWithBlockNumber,
    current_head_number: u64,
    l1_message_delay: u64,
) -> bool {
    let tx_block_number = msg_w_bn.block_number;
    let delay = current_head_number.saturating_sub(tx_block_number);
    delay >= l1_message_delay
}

#[async_trait::async_trait]
impl<DB: DatabaseConnectionProvider + Sync> L1MessageProvider
    for DatabaseL1MessageDelayProvider<DB>
{
    type Error = L1ProviderError;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageWithBlockNumber>, Self::Error> {
        let msg_w_bn = self.l1_message_provider.get_l1_message_with_block_number().await?;
        if let Some(msg_w_bn) = msg_w_bn {
            if validate_delay_predicate(&msg_w_bn, self.l1_head, self.l1_message_delay) {
                return Ok(Some(msg_w_bn));
            }
        }

        Ok(None)
    }

    fn set_index_cursor(&self, index: u64) {
        self.l1_message_provider.set_index_cursor(index);
    }

    fn set_hash_cursor(&self, _hash: B256) {
        // TODO: issue 43
        todo!()
    }

    fn increment_cursor(&self) {
        self.l1_message_provider.increment_cursor();
    }
}
