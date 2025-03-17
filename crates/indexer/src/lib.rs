//! A library responsible for indexing data relevant to the L1.
use alloy_primitives::B256;
use reth_tokio_util::{EventSender, EventStream};
use rollup_node_primitives::{BatchInput, L1MessageWithBlockNumber};
use rollup_node_watcher::L1Notification;
use scroll_db::{Database, DbErr};
use std::sync::Arc;
use tokio::sync::mpsc;

mod event;
pub use event::IndexerEvent;

mod handle;
use handle::IndexerCommand;
pub use handle::IndexerHandle;

/// The indexer is responsible for indexing data relevant to the rollup node.
#[derive(Debug)]
pub struct Indexer {
    /// A reference to the database used to persist the indexed data.
    database: Arc<Database>,
    /// A channel to receive commands to index data.
    cmd_rx: mpsc::UnboundedReceiver<IndexerCommand>,
    /// An event sender for sending events to subscribers of the indexer.
    event_sender: Option<EventSender<IndexerEvent>>,
}

impl Indexer {
    /// Creates a new indexer with the given database and returns the [`IndexerHandle`] .
    pub fn spawn(database: Arc<Database>) -> IndexerHandle {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let indexer = Indexer { database, cmd_rx, event_sender: None };
        tokio::spawn(indexer.run());
        IndexerHandle::new(cmd_tx)
    }

    /// Creates a new indexer with the given database and returns an [`IndexerHandle`] and
    /// [`EventStream<IndexerEvent>`].
    pub fn spawn_with_event_stream(
        database: Arc<Database>,
    ) -> (IndexerHandle, EventStream<IndexerEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let event_sender = EventSender::new(2000);
        let event_listener = event_sender.new_listener();
        let indexer = Indexer { database, cmd_rx, event_sender: Some(event_sender) };
        tokio::spawn(indexer.run());
        (IndexerHandle::new(cmd_tx), event_listener)
    }

    /// The main loop of the indexer.
    async fn run(mut self) {
        loop {
            if let Some(IndexerCommand::IndexL1Notification(event)) = self.cmd_rx.recv().await {
                if let Err(err) = self.handle_l1_event(event).await {
                    tracing::error!(target: "rollup_node_indexer", "error handling L1 event {:?}", err);
                }
            } else {
                tracing::trace!(target: "rollup_node_indexer", "indexer command channel closed - shutting down");
                break;
            }
        }
    }

    /// Handles an event from the L1.
    pub async fn handle_l1_event(&self, event: L1Notification) -> Result<(), DbErr> {
        let result = match event {
            L1Notification::Reorg(block_number) => self.handle_reorg(block_number).await,
            L1Notification::NewBlock(_block_number) | L1Notification::Finalized(_block_number) => {
                Ok(())
            }
            L1Notification::BatchCommit(batch_input) => self.handle_batch_input(batch_input).await,
            L1Notification::L1Message(l1_message) => self.handle_l1_message(l1_message).await,
            L1Notification::BatchFinalization { hash, block_number } => {
                self.handle_batch_finalization(hash, block_number).await
            }
        };

        if let Some(event_sender) = &self.event_sender {
            event_sender.notify(IndexerEvent::L1NotificationIndexed);
        }

        result
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

    /// Handles an L1 message by inserting it into the database.
    async fn handle_l1_message(&self, l1_message: L1MessageWithBlockNumber) -> Result<(), DbErr> {
        self.database.insert_l1_message(self.database.connection(), l1_message).await.map(|_| ())
    }

    /// Handles a batch input by inserting it into the database.
    async fn handle_batch_input(&self, batch_input: BatchInput) -> Result<(), DbErr> {
        self.database.insert_batch_input(self.database.connection(), batch_input).await.map(|_| ())
    }

    /// Handles a batch finalization event by updating the batch input in the database.
    async fn handle_batch_finalization(
        &self,
        batch_hash: B256,
        block_number: u64,
    ) -> Result<(), DbErr> {
        self.database
            .finalize_batch_input(self.database.connection(), batch_hash, block_number)
            .await
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

    async fn setup_test_indexer() -> (IndexerHandle, EventStream<IndexerEvent>, Arc<Database>) {
        let db = Arc::new(setup_test_db().await);
        let (handle, event_stream) = Indexer::spawn_with_event_stream(db.clone());
        (handle, event_stream, db)
    }

    #[tokio::test]
    async fn test_handle_commit_batch() {
        // Instantiate indexer and db
        let (handle, mut event_stream, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let batch_input = BatchInput::arbitrary(&mut u).unwrap();
        handle.index_l1_notification(L1Notification::BatchCommit(batch_input.clone()));

        let _ = event_stream.next().await;

        let batch_input_result = db
            .get_batch_input_by_batch_index(db.connection(), batch_input.batch_index())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(batch_input, batch_input_result);
    }

    #[tokio::test]
    async fn test_handle_l1_message() {
        // Instantiate indexer and db
        let (handle, mut event_stream, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let l1_message = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();
        handle.index_l1_notification(L1Notification::L1Message(l1_message.clone()));

        let _ = event_stream.next().await;

        let l1_message_result =
            db.get_l1_message(l1_message.transaction.queue_index).await.unwrap().unwrap();

        assert_eq!(l1_message, l1_message_result);
    }

    #[tokio::test]
    async fn test_handle_reorg() {
        // Instantiate indexer and db
        let (handle, mut event_stream, db) = setup_test_indexer().await;

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
        handle.index_l1_notification(L1Notification::BatchCommit(batch_input_block_1.clone()));
        handle.index_l1_notification(L1Notification::BatchCommit(batch_input_block_20.clone()));
        handle.index_l1_notification(L1Notification::BatchCommit(batch_input_block_30.clone()));

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
        handle.index_l1_notification(L1Notification::L1Message(l1_message_block_1.clone()));
        handle.index_l1_notification(L1Notification::L1Message(l1_message_block_20.clone()));
        handle.index_l1_notification(L1Notification::L1Message(l1_message_block_30.clone()));

        // Reorg at block 20
        handle.index_l1_notification(L1Notification::Reorg(20));

        for _ in 0..7 {
            event_stream.next().await;
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
