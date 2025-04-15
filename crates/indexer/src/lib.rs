//! A library responsible for indexing data relevant to the L1.
use alloy_primitives::B256;
use futures::Stream;
use rollup_node_primitives::{BatchCommitData, BatchInfo, BlockInfo, L1MessageWithBlockNumber};
use rollup_node_watcher::L1Notification;
use scroll_db::{Database, DatabaseOperations};
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::Mutex;

mod action;
use action::IndexerFuture;

mod event;
pub use event::IndexerEvent;

mod error;
pub use error::IndexerError;

/// The indexer is responsible for indexing data relevant to the L1.
#[derive(Debug)]
pub struct Indexer {
    /// A reference to the database used to persist the indexed data.
    database: Arc<Database>,
    /// A queue of pending futures.
    pending_futures: VecDeque<IndexerFuture>,
    /// The block number of the L1 finalized block.
    l1_finalized_block_number: Arc<Mutex<u64>>,
    /// The block number of the L2 finalized block.
    l2_finalized_block_number: Arc<Mutex<u64>>,
}

impl Indexer {
    /// Creates a new indexer with the given [`Database`].
    pub fn new(database: Arc<Database>) -> Self {
        Self {
            database,
            pending_futures: Default::default(),
            l1_finalized_block_number: Arc::new(Mutex::new(0)),
            l2_finalized_block_number: Arc::new(Mutex::new(0)),
        }
    }

    /// Handles a new batch to L2 block.
    pub fn handle_batch_to_block(&mut self, batch_info: BatchInfo, block_info: BlockInfo) {
        let database = self.database.clone();
        let fut = IndexerFuture::HandleBatchToBlock(Box::pin(async move {
            database.insert_batch_to_block(batch_info, block_info).await?;
            Result::<_, IndexerError>::Ok(IndexerEvent::BatchToBlockIndexed(batch_info, block_info))
        }));
        self.pending_futures.push_back(fut)
    }

    /// Handles an event from the L1.
    pub fn handle_l1_notification(&mut self, event: L1Notification) {
        let fut = match event {
            L1Notification::Reorg(block_number) => IndexerFuture::HandleReorg(Box::pin(
                Self::handle_reorg(self.database.clone(), block_number),
            )),
            L1Notification::NewBlock(_block_number) => return,
            L1Notification::Finalized(block_number) => {
                IndexerFuture::HandleFinalized(Box::pin(Self::handle_finalized(
                    self.database.clone(),
                    block_number,
                    self.l1_finalized_block_number.clone(),
                    self.l2_finalized_block_number.clone(),
                )))
            }
            L1Notification::BatchCommit(batch) => IndexerFuture::HandleBatchCommit(Box::pin(
                Self::handle_batch_commit(self.database.clone(), batch),
            )),
            L1Notification::L1Message(l1_message) => IndexerFuture::HandleL1Message(Box::pin(
                Self::handle_l1_message(self.database.clone(), l1_message),
            )),
            L1Notification::BatchFinalization { hash, block_number } => {
                IndexerFuture::HandleBatchFinalization(Box::pin(Self::handle_batch_finalization(
                    self.database.clone(),
                    hash,
                    block_number,
                    self.l1_finalized_block_number.clone(),
                    self.l2_finalized_block_number.clone(),
                )))
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
        txn.delete_batches_gt(block_number).await?;
        txn.delete_l1_messages_gt(block_number).await?;

        // commit the transaction
        txn.commit().await?;
        Ok(IndexerEvent::ReorgIndexed(block_number))
    }

    /// Handles a finalized event by updating the indexer L1 finalized block and returning the new
    /// finalized L2 chain block.
    async fn handle_finalized(
        database: Arc<Database>,
        block_number: u64,
        l1_block_number: Arc<Mutex<u64>>,
        l2_block_number: Arc<Mutex<u64>>,
    ) -> Result<IndexerEvent, IndexerError> {
        // get the newest finalized batch.
        let batch_hash = database.get_finalized_batch_hash_at_height(block_number).await?;

        // get the finalized block for the batch.
        let finalized_block = if let Some(hash) = batch_hash {
            Self::fetch_highest_finalized_block(database, hash, l2_block_number).await?
        } else {
            None
        };

        // update the indexer l1 block number.
        *l1_block_number.lock().await = block_number;

        Ok(IndexerEvent::FinalizedIndexed(block_number, finalized_block))
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
        batch: BatchCommitData,
    ) -> Result<IndexerEvent, IndexerError> {
        let event = IndexerEvent::BatchCommitIndexed(BatchInfo::new(batch.index, batch.hash));
        database.insert_batch(batch).await?;
        Ok(event)
    }

    /// Handles a batch finalization event by updating the batch input in the database.
    async fn handle_batch_finalization(
        database: Arc<Database>,
        batch_hash: B256,
        block_number: u64,
        l1_block_number: Arc<Mutex<u64>>,
        l2_block_number: Arc<Mutex<u64>>,
    ) -> Result<IndexerEvent, IndexerError> {
        // finalized the batch.
        database.finalize_batch(batch_hash, block_number).await?;

        // check if the block where the batch was finalized is finalized on L1.
        let mut finalized_block = None;
        let l1_block_number = *l1_block_number.lock().await;
        if l1_block_number > block_number {
            // fetch the finalized block.
            finalized_block =
                Self::fetch_highest_finalized_block(database, batch_hash, l2_block_number).await?;
        }

        let event = IndexerEvent::BatchFinalizationIndexed(batch_hash, finalized_block);
        Ok(event)
    }

    /// Returns the highest finalized block for the provided batch hash. Will return [`None`] if the
    /// block number has already been seen by the indexer.
    async fn fetch_highest_finalized_block(
        database: Arc<Database>,
        batch_hash: B256,
        l2_block_number: Arc<Mutex<u64>>,
    ) -> Result<Option<BlockInfo>, IndexerError> {
        let finalized_block = database.get_highest_block_for_batch(batch_hash).await?;
        let mut l2_block_number = l2_block_number.lock().await;

        // only return the block if the indexer hasn't seen it.
        // in which case also update the `l2_finalized_block_number` value.
        Ok(finalized_block.filter(|info| info.number > *l2_block_number).inspect(|info| {
            *l2_block_number = info.number;
        }))
    }
}

impl Stream for Indexer {
    type Item = Result<IndexerEvent, IndexerError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Remove and poll the next future in the queue
        if let Some(mut action) = self.pending_futures.pop_front() {
            return match action.poll(cx) {
                Poll::Ready(result) => Poll::Ready(Some(result)),
                Poll::Pending => {
                    self.pending_futures.push_front(action);
                    Poll::Pending
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
    use rollup_node_primitives::BatchCommitData;
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

        let batch_commit = BatchCommitData::arbitrary(&mut u).unwrap();
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit.clone()));

        let _ = indexer.next().await;

        let batch_commit_result = db.get_batch_by_index(batch_commit.index).await.unwrap().unwrap();

        assert_eq!(batch_commit, batch_commit_result);
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
        let mut batch_commit_block_1 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_1.block_number = 1;
        let batch_commit_block_1 = batch_commit_block_1;

        let mut batch_commit_block_20 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_20.block_number = 20;
        let batch_commit_block_20 = batch_commit_block_20;

        let mut batch_commit_block_30 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_30.block_number = 30;
        let batch_commit_block_30 = batch_commit_block_30;

        // Index batch inputs
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_20.clone()));
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_30.clone()));

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
            indexer.next().await.unwrap().unwrap();
        }

        // Check that the batch input at block 30 is deleted
        let batch_commits =
            db.get_batches().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;

        assert_eq!(2, batch_commits.len());
        assert!(batch_commits.contains(&batch_commit_block_1));
        assert!(batch_commits.contains(&batch_commit_block_20));

        // check that the L1 message at block 30 is deleted
        let l1_messages =
            db.get_l1_messages().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;
        assert_eq!(2, l1_messages.len());
        assert!(l1_messages.contains(&l1_message_block_1));
        assert!(l1_messages.contains(&l1_message_block_20));
    }
}
