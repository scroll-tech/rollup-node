use crate::L1ProviderError;

use alloy_primitives::B256;
use rollup_node_primitives::L1MessageEnvelope;
use scroll_alloy_consensus::TxL1Message;
use scroll_db::{DatabaseConnectionProvider, DatabaseOperations};

mod database;
pub use database::{
    DatabaseL1MessageDelayProvider, DatabaseL1MessageProvider, L1MessageDelayProvider,
};

/// An instance of the trait can provide L1 messages using a cursor approach. Set the cursor for the
/// provider using the queue index or hash and then call
/// [`L1MessageProvider::next_l1_message_with_block_number`] to iterate the queue.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait L1MessageProvider {
    /// The error type for the provider.
    type Error: Into<L1ProviderError>;

    /// Returns the L1 message with block number at the current cursor.
    /// This method does not advance the cursor.
    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageEnvelope>, Self::Error>;

    /// Returns the L1 message with block number at the current cursor and advances the cursor.
    async fn next_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageEnvelope>, Self::Error> {
        if let Some(message) = self.get_l1_message_with_block_number().await? {
            self.increment_cursor();
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    /// Returns the L1 message with block number at the current cursor and advances the cursor.
    async fn next_l1_message(&self) -> Result<Option<TxL1Message>, Self::Error> {
        let message = self.next_l1_message_with_block_number().await?;
        Ok(message.map(|message| message.transaction))
    }

    /// Returns the L1 message at the current cursor.
    /// This method does not advance the cursor.
    async fn get_l1_message(&self) -> Result<Option<TxL1Message>, Self::Error> {
        let message = self.get_l1_message_with_block_number().await?;
        Ok(message.map(|message| message.transaction))
    }
    /// Set the index cursor for the provider.
    fn set_index_cursor(&self, index: u64);
    /// Set the hash cursor for the provider.
    fn set_hash_cursor(&self, hash: B256);
    /// Increment cursor index.
    fn increment_cursor(&self);
}
