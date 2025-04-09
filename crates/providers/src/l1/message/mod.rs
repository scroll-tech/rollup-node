use crate::L1ProviderError;

use alloy_primitives::B256;
use rollup_node_primitives::L1MessageWithBlockNumber;
use scroll_alloy_consensus::TxL1Message;
use scroll_db::{DatabaseConnectionProvider, DatabaseOperations};

mod database;
pub use database::{DatabaseL1MessageDelayProvider, DatabaseL1MessageProvider};

/// An instance of the trait can provide L1 messages using a cursor approach. Set the cursor for the
/// provider using the queue index or hash and then call
/// [`L1MessageProvider::next_l1_message_with_block_number`] to iterate the queue.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait L1MessageWithBlockNumberProvider {
    /// The error type for the provider.
    type Error: Into<L1ProviderError>;

    /// Returns the L1 message with block number at the current cursor and advances the cursor.
    async fn next_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageWithBlockNumber>, Self::Error> {
        if let Some(message) = self.get_l1_message_with_block_number().await? {
            self.increment_cursor();
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }
    /// Returns the L1 message with block number at the current cursor.
    /// This method does not advance the cursor.
    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageWithBlockNumber>, Self::Error>;
    /// Set the index cursor for the provider.
    fn set_index_cursor(&self, index: u64);
    /// Set the hash cursor for the provider.
    fn set_hash_cursor(&self, hash: B256);
    /// Increment cursor index.
    fn increment_cursor(&self);
}

/// An instance of the trait can provide L1 messages using a cursor approach. Set the cursor for the
/// provider using the queue index or hash and then call [`L1MessageProvider::next_l1_message`] to
/// iterate the queue.
#[async_trait::async_trait]
pub trait L1MessageProvider: L1MessageWithBlockNumberProvider {
    /// Returns the L1 message at the current cursor and advances the cursor.
    async fn next_l1_message(&mut self) -> Result<Option<TxL1Message>, Self::Error> {
        let message = self.next_l1_message_with_block_number().await?;
        Ok(message.map(|message| message.transaction))
    }

    /// Returns the L1 message at the current cursor.
    /// This method does not advance the cursor.
    async fn get_l1_message(&self) -> Result<Option<TxL1Message>, Self::Error> {
        let message = self.get_l1_message_with_block_number().await?;
        Ok(message.map(|message| message.transaction))
    }
}

/// A blanket implementation of [`L1MessageProvider`] for any type that implements
/// [`L1MessageWithBlockNumberProvider`].
impl<T> L1MessageProvider for T where T: L1MessageWithBlockNumberProvider + Sync {}

pub trait L1MessageProviderWithPredicate: L1MessageWithBlockNumberProvider {
    async fn next_l1_message_with_predicate(
        &self,
        predicate: impl Fn(&L1MessageWithBlockNumber, u64) -> bool + Send,
    ) -> Result<Option<L1MessageWithBlockNumber>, Self::Error> {
        loop {
            let message = self.next_l1_message_with_block_number().await?;
            if let Some(message) = message {
                if predicate(&message) {
                    return Ok(Some(message));
                }
            } else {
                return Ok(None);
            }
        }
    }
}
