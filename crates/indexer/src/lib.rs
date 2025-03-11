//! A library responsible for indexing data relevant to the L1.

use scroll_db::{Database, DbErr};
use scroll_l1::L1Event;
use scroll_primitives::{BatchInput, L1Message};
use std::sync::Arc;

/// The indexer is responsible for indexing data relevant to the L1.
#[derive(Debug)]
pub struct Indexer {
    database: Arc<Database>,
}

impl Indexer {
    /// Creates a new indexer with the given database.
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }

    /// Handles an event from the L1.
    pub async fn handle_l1_event(&self, event: L1Event) -> Result<(), DbErr> {
        match event {
            L1Event::CommitBatch(batch_input) => self.handle_batch_input(batch_input).await,
            L1Event::Reorg(block_number) => self.handle_reorg(block_number).await,
            L1Event::NewBlock(_block_number) => Ok(()),
            L1Event::Finalized(block_number) => self.handle_finalized(block_number).await,
            L1Event::L1Message(l1_message) => self.handle_l1_message(l1_message).await,
        }
    }

    /// Handles a reorganization event by deleting all indexed data which is greater than the
    /// provided block number.
    async fn handle_reorg(&self, block_number: u64) -> Result<(), DbErr> {
        // create a database transaction so this operation is atomic
        let txn = self.database.tx().await?;

        // delete batch inputs and l1 messages
        self.database.delete_batch_inputs_gt(&txn, block_number).await?;
        self.database.delete_l1_messages_gt(&txn, block_number).await?;

        // commit the transaction
        txn.commit().await?;
        Ok(())
    }

    async fn handle_finalized(&self, _block_number: u64) -> Result<(), DbErr> {
        todo!()
    }

    /// Handles an L1 message by inserting it into the database.
    async fn handle_l1_message(&self, l1_message: Arc<L1Message>) -> Result<(), DbErr> {
        self.database
            .insert_l1_message(self.database.connection(), (*l1_message).clone())
            .await
            .map(|_| ())
    }

    /// Handles a batch input by inserting it into the database.
    async fn handle_batch_input(&self, batch_input: Arc<BatchInput>) -> Result<(), DbErr> {
        self.database
            .insert_batch_input(self.database.connection(), (*batch_input).clone())
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use arbitrary::{Arbitrary, Unstructured};
    use rand::Rng;
    use scroll_db::{batch_input, test_utils::setup_test_db, DatabaseEntryStream};
    use scroll_primitives::BatchInput;

    async fn setup_test_indexer() -> (Indexer, Arc<Database>) {
        let db = Arc::new(setup_test_db().await);
        (Indexer::new(db.clone()), db)
    }

    #[tokio::test]
    async fn test_handle_commit_batch() {
        // Instantiate indexer and db
        let (indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::thread_rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let batch_input = Arc::new(BatchInput::arbitrary(&mut u).unwrap());
        indexer.handle_batch_input(batch_input.clone()).await.unwrap();

        let batch_input_result = db
            .get_batch_input_by_batch_index(db.connection(), batch_input.batch_index())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(*batch_input, batch_input_result);
    }

    #[tokio::test]
    async fn test_handle_l1_message() {
        // Instantiate indexer and db
        let (indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::thread_rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let l1_message = Arc::new(L1Message::arbitrary(&mut u).unwrap());
        indexer.handle_l1_message(l1_message.clone()).await.unwrap();

        let l1_message_result =
            db.get_l1_message(l1_message.transaction.queue_index).await.unwrap().unwrap();

        assert_eq!(*l1_message, l1_message_result);
    }

    #[tokio::test]
    async fn test_handle_reorg() {
        // Instantiate indexer and db
        let (indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::thread_rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a 3 random batch inputs and set their block numbers
        let mut batch_input_block_1 = BatchInput::arbitrary(&mut u).unwrap();
        batch_input_block_1.set_block_number(1);
        let batch_input_block_1 = Arc::new(batch_input_block_1);

        let mut batch_input_block_20 = BatchInput::arbitrary(&mut u).unwrap();
        batch_input_block_20.set_block_number(20);
        let batch_input_block_20 = Arc::new(batch_input_block_20);

        let mut batch_input_block_30 = BatchInput::arbitrary(&mut u).unwrap();
        batch_input_block_30.set_block_number(30);
        let batch_input_block_30 = Arc::new(batch_input_block_30);

        // Index batch inputs
        indexer.handle_batch_input(batch_input_block_1.clone()).await.unwrap();
        indexer.handle_batch_input(batch_input_block_20.clone()).await.unwrap();
        indexer.handle_batch_input(batch_input_block_30.clone()).await.unwrap();

        // Generate 3 random L1 messages and set their block numbers
        let mut l1_message_block_1 = L1Message::arbitrary(&mut u).unwrap();
        l1_message_block_1.block_number = 1;
        let l1_message_block_1 = Arc::new(l1_message_block_1);

        let mut l1_message_block_20 = L1Message::arbitrary(&mut u).unwrap();
        l1_message_block_20.block_number = 20;
        let l1_message_block_20 = Arc::new(l1_message_block_20);

        let mut l1_message_block_30 = L1Message::arbitrary(&mut u).unwrap();
        l1_message_block_30.block_number = 30;
        let l1_message_block_30 = Arc::new(l1_message_block_30);

        // Index L1 messages
        indexer.handle_l1_message(l1_message_block_1.clone()).await.unwrap();
        indexer.handle_l1_message(l1_message_block_20.clone()).await.unwrap();
        indexer.handle_l1_message(l1_message_block_30.clone()).await.unwrap();

        // Reorg at block 20
        indexer.handle_reorg(20).await.unwrap();

        // Check that the batch input at block 30 is deleted
        let mut batch_input_iter = db.get_batch_inputs().await.unwrap();
        let mut batch_inputs = vec![];
        while let Some(batch_input) = batch_input_iter.next_entry().await {
            batch_inputs.push(batch_input.unwrap());
        }
        assert_eq!(2, batch_inputs.len());
        assert!(batch_inputs.contains(&batch_input_block_1));
        assert!(batch_inputs.contains(&batch_input_block_20));

        // check that the L1 message at block 30 is deleted
        let mut l1_message_iter = db.get_l1_messages().await.unwrap();
        let mut l1_messages = vec![];
        while let Some(l1_message) = l1_message_iter.next_entry().await {
            l1_messages.push(l1_message.unwrap());
        }
        assert_eq!(2, l1_messages.len());
        assert!(l1_messages.contains(&l1_message_block_1));
        assert!(l1_messages.contains(&l1_message_block_20));
    }
}
