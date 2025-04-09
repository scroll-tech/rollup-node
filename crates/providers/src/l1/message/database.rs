use super::*;

/// Implements [`L1MessageProvider`] via a database connection.
#[derive(Debug)]
pub struct DatabaseL1MessageProvider<DB> {
    /// A connection to the database.
    database_connection: DB,
    /// The current L1 message index.
    index: u64,
}

impl<DB> DatabaseL1MessageProvider<DB> {
    /// Returns a new instance of the [`DatabaseL1MessageProvider`].
    pub const fn new(db: DB, index: u64) -> Self {
        Self { database_connection: db, index }
    }
}

#[async_trait::async_trait]
impl<DB: DatabaseConnectionProvider + Sync> L1MessageWithBlockNumberProvider
    for DatabaseL1MessageProvider<DB>
{
    type Error = L1ProviderError;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageWithBlockNumber>, Self::Error> {
        Ok(self.database_connection.get_l1_message(self.index).await?)
    }

    fn set_index_cursor(&mut self, index: u64) {
        self.index = index;
    }

    fn set_hash_cursor(&mut self, _hash: B256) {
        // TODO: issue 43
        todo!()
    }

    fn increment_cursor(&mut self) {
        self.index += 1;
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
    current_head_number: u64,
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
        Self { l1_message_provider, current_head_number, l1_message_delay }
    }

    /// Sets the block number of the current L1 head.
    pub fn set_current_head_number(&mut self, current_head_number: u64) {
        self.current_head_number = current_head_number;
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
impl<DB: DatabaseConnectionProvider + Sync> L1MessageWithBlockNumberProvider
    for DatabaseL1MessageDelayProvider<DB>
{
    type Error = L1ProviderError;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageWithBlockNumber>, Self::Error> {
        let msg_w_bn = self.l1_message_provider.get_l1_message_with_block_number().await?;
        if let Some(msg_w_bn) = msg_w_bn {
            if validate_delay_predicate(&msg_w_bn, self.current_head_number, self.l1_message_delay)
            {
                return Ok(Some(msg_w_bn));
            }
        }

        Ok(None)
    }

    fn set_index_cursor(&mut self, index: u64) {
        self.l1_message_provider.set_index_cursor(index);
    }

    fn set_hash_cursor(&mut self, _hash: B256) {
        // TODO: issue 43
        todo!()
    }

    fn increment_cursor(&mut self) {
        self.l1_message_provider.increment_cursor();
    }
}
