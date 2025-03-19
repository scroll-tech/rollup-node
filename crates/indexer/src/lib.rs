//! A library responsible for indexing data relevant to the L1.
use alloy_primitives::B256;
use futures::Stream;
use rollup_node_primitives::{BatchInput, L1MessageWithBlockNumber};
use rollup_node_watcher::L1Notification;
use scroll_db::{Database, DatabaseOperations};
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

mod action;
use action::IndexerAction;

mod event;
pub use event::IndexerEvent;

mod error;
use error::IndexerError;

/// The indexer is responsible for indexing data relevant to the L1.
#[derive(Debug)]
pub struct Indexer {
    /// A reference to the database used to persist the indexed data.
    database: Arc<Database>,
    /// A queue of pending futures.
    pending_futures: VecDeque<IndexerAction>,
}

impl Indexer {
    /// Creates a new indexer with the given [`Database`].
    pub fn new(database: Arc<Database>) -> Self {
        Self { database, pending_futures: Default::default() }
    }

    /// Handles an event from the L1.
    pub fn handle_l1_notification(&mut self, event: L1Notification) {
        let fut =
            match event {
                L1Notification::Reorg(block_number) => IndexerAction::HandleReorg(Box::pin(
                    Self::handle_reorg(self.database.clone(), block_number),
                )),
                L1Notification::NewBlock(_block_number) |
                L1Notification::Finalized(_block_number) => return,
                L1Notification::BatchCommit(batch_input) => IndexerAction::HandleBatchCommit(
                    Box::pin(Self::handle_batch_commit(self.database.clone(), batch_input)),
                ),
                L1Notification::L1Message(l1_message) => IndexerAction::HandleL1Message(Box::pin(
                    Self::handle_l1_message(self.database.clone(), l1_message),
                )),
                L1Notification::BatchFinalization { hash, block_number } => {
                    IndexerAction::HandleBatchFinalization(Box::pin(
                        Self::handle_batch_finalization(self.database.clone(), hash, block_number),
                    ))
                }
            };

        self.pending_futures.push_back(fut);
    }

    /// Handles a reorganization event by deleting all indexed data which is greater than the
    /// provided block number.
    async fn handle_reorg(
        database: Arc<Database>,
        block_number: u64,
    ) -> Result<IndexerEvent, IndexerError> {
        // create a database transaction so this operation is atomic
        let txn = database.tx().await?;

        // delete batch inputs and l1 messages
        txn.delete_batch_inputs_gt(block_number).await?;
        txn.delete_l1_messages_gt(block_number).await?;

        // commit the transaction
        txn.commit().await?;
        Ok(IndexerEvent::ReorgIndexed(block_number))
    }

    /// Handles an L1 message by inserting it into the database.
    async fn handle_l1_message(
        database: Arc<Database>,
        l1_message: L1MessageWithBlockNumber,
    ) -> Result<IndexerEvent, IndexerError> {
        let event = IndexerEvent::L1MessageIndexed(l1_message.transaction.queue_index);
        database.insert_l1_message(l1_message).await?;
        Ok(event)
    }

    /// Handles a batch input by inserting it into the database.
    async fn handle_batch_commit(
        database: Arc<Database>,
        batch_input: BatchInput,
    ) -> Result<IndexerEvent, IndexerError> {
        let event = IndexerEvent::BatchCommitIndexed(batch_input.batch_index());
        database.insert_batch_input(batch_input).await?;
        Ok(event)
    }

    /// Handles a batch finalization event by updating the batch input in the database.
    async fn handle_batch_finalization(
        database: Arc<Database>,
        batch_hash: B256,
        block_number: u64,
    ) -> Result<IndexerEvent, IndexerError> {
        let event = IndexerEvent::BatchFinalizationIndexed(block_number);
        database.finalize_batch_input(batch_hash, block_number).await?;
        Ok(event)
    }
}

impl Stream for Indexer {
    type Item = Result<IndexerEvent, IndexerError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Remove and poll the next future in the queue
        if let Some(mut action) = self.pending_futures.pop_front() {
            match action.poll(cx) {
                Poll::Ready(result) => return Poll::Ready(Some(result)),
                Poll::Pending => {
                    self.pending_futures.push_front(action);
                    return Poll::Pending;
                }
            }
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use arbitrary::{Arbitrary, Unstructured};
    use futures::StreamExt;
    use rand::Rng;
    use rollup_node_primitives::BatchInput;
    use scroll_db::test_utils::setup_test_db;

    async fn setup_test_indexer() -> (Indexer, Arc<Database>) {
        let db = Arc::new(setup_test_db().await);
        (Indexer::new(db.clone()), db)
    }

    #[tokio::test]
    async fn test_handle_commit_batch() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let batch_input = BatchInput::arbitrary(&mut u).unwrap();
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_input.clone()));

        let _ = indexer.next().await;

        let batch_input_result =
            db.get_batch_input_by_batch_index(batch_input.batch_index()).await.unwrap().unwrap();

        assert_eq!(batch_input, batch_input_result);
    }

    #[tokio::test]
    async fn test_handle_l1_message() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let l1_message = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();
        indexer.handle_l1_notification(L1Notification::L1Message(l1_message.clone()));

        let _ = indexer.next().await;

        let l1_message_result =
            db.get_l1_message(l1_message.transaction.queue_index).await.unwrap().unwrap();

        assert_eq!(l1_message, l1_message_result);
    }

    #[tokio::test]
    async fn test_handle_reorg() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a 3 random batch inputs and set their block numbers
        let mut batch_input_block_1 = BatchInput::arbitrary(&mut u).unwrap();
        batch_input_block_1.set_block_number(1);
        let batch_input_block_1 = batch_input_block_1;

        let mut batch_input_block_20 = BatchInput::arbitrary(&mut u).unwrap();
        batch_input_block_20.set_block_number(20);
        let batch_input_block_20 = batch_input_block_20;

        let mut batch_input_block_30 = BatchInput::arbitrary(&mut u).unwrap();
        batch_input_block_30.set_block_number(30);
        let batch_input_block_30 = batch_input_block_30;

        // Index batch inputs
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_input_block_1.clone()));
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_input_block_20.clone()));
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_input_block_30.clone()));

        // Generate 3 random L1 messages and set their block numbers
        let mut l1_message_block_1 = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();
        l1_message_block_1.block_number = 1;
        let l1_message_block_1 = l1_message_block_1;

        let mut l1_message_block_20 = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();
        l1_message_block_20.block_number = 20;
        let l1_message_block_20 = l1_message_block_20;

        let mut l1_message_block_30 = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();
        l1_message_block_30.block_number = 30;
        let l1_message_block_30 = l1_message_block_30;

        // Index L1 messages
        indexer.handle_l1_notification(L1Notification::L1Message(l1_message_block_1.clone()));
        indexer.handle_l1_notification(L1Notification::L1Message(l1_message_block_20.clone()));
        indexer.handle_l1_notification(L1Notification::L1Message(l1_message_block_30.clone()));

        // Reorg at block 20
        indexer.handle_l1_notification(L1Notification::Reorg(20));

        for _ in 0..7 {
            indexer.next().await;
        }

        // Check that the batch input at block 30 is deleted
        let batch_inputs =
            db.get_batch_inputs().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;

        assert_eq!(2, batch_inputs.len());
        assert!(batch_inputs.contains(&batch_input_block_1));
        assert!(batch_inputs.contains(&batch_input_block_20));

        // check that the L1 message at block 30 is deleted
        let l1_messages =
            db.get_l1_messages().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;
        assert_eq!(2, l1_messages.len());
        assert!(l1_messages.contains(&l1_message_block_1));
        assert!(l1_messages.contains(&l1_message_block_20));
    }
}
