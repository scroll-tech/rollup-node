//! A library responsible for indexing data relevant to the L1.

use alloy_primitives::{b256, keccak256, B256};
use futures::Stream;
use reth_chainspec::EthChainSpec;
use rollup_node_primitives::{
    BatchCommitData, BatchInfo, BlockInfo, L1MessageEnvelope, L2BlockInfoWithL1Messages,
};
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_hardforks::{ScrollHardfork, ScrollHardforks};
use scroll_db::{Database, DatabaseError, DatabaseOperations};
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

mod action;
use action::IndexerFuture;

mod event;
pub use event::IndexerEvent;

mod error;
pub use error::IndexerError;

const L1_MESSAGE_QUEUE_HASH_MASK: B256 =
    b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000");

/// The indexer is responsible for indexing data relevant to the L1.
#[derive(Debug)]
pub struct Indexer<ChainSpec> {
    /// A reference to the database used to persist the indexed data.
    database: Arc<Database>,
    /// A queue of pending futures.
    pending_futures: VecDeque<IndexerFuture>,
    /// The block number of the L1 finalized block.
    l1_finalized_block_number: Arc<AtomicU64>,
    /// The block number of the L2 finalized block.
    l2_finalized_block_number: Arc<AtomicU64>,
    /// The chain specification for the indexer.
    chain_spec: Arc<ChainSpec>,
}

impl<ChainSpec: ScrollHardforks + EthChainSpec + Send + Sync + 'static> Indexer<ChainSpec> {
    /// Creates a new indexer with the given [`Database`].
    pub fn new(database: Arc<Database>, chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            database,
            pending_futures: Default::default(),
            l1_finalized_block_number: Arc::new(AtomicU64::new(0)),
            l2_finalized_block_number: Arc::new(AtomicU64::new(0)),
            chain_spec,
        }
    }

    /// Handles an L2 block.
    pub fn handle_block(
        &mut self,
        block_info: L2BlockInfoWithL1Messages,
        batch_info: Option<BatchInfo>,
    ) {
        let database = self.database.clone();
        let fut = IndexerFuture::HandleDerivedBlock(Box::pin(async move {
            database.insert_block(block_info.clone(), batch_info).await?;
            Result::<_, IndexerError>::Ok(IndexerEvent::BlockIndexed(block_info, batch_info))
        }));
        self.pending_futures.push_back(fut)
    }

    /// Unwinds a batch by deleting all data that was indexed after the block number the batch was
    /// created.
    pub fn unwind_batch(&mut self, batch_info: BatchInfo) {
        let fut = IndexerFuture::HandleUnwindBatch(Box::pin(Self::handle_unwind_batch(
            self.database.clone(),
            self.chain_spec.clone(),
            batch_info,
        )));
        self.pending_futures.push_back(fut);
    }

    async fn handle_unwind_batch(
        database: Arc<Database>,
        chain_spec: Arc<ChainSpec>,
        batch_info: BatchInfo,
    ) -> Result<IndexerEvent, IndexerError> {
        let batch = database.get_batch_by_index(batch_info.index).await?.expect("batch must exist");
        Self::unwind(database, chain_spec, batch.block_number - 1).await
    }

    /// Handles an event from the L1.
    pub fn handle_l1_notification(&mut self, event: L1Notification) {
        let fut = match event {
            L1Notification::Reorg(block_number) => IndexerFuture::HandleUnwind(Box::pin(
                Self::unwind(self.database.clone(), self.chain_spec.clone(), block_number),
            )),
            L1Notification::NewBlock(_) | L1Notification::Consensus(_) => return,
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
            L1Notification::L1Message { message, block_number, block_timestamp } => {
                IndexerFuture::HandleL1Message(Box::pin(Self::handle_l1_message(
                    self.database.clone(),
                    self.chain_spec.clone(),
                    message,
                    block_number,
                    block_timestamp,
                )))
            }
            L1Notification::BatchFinalization { hash, block_number, .. } => {
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

    async fn unwind(
        database: Arc<Database>,
        chain_spec: Arc<ChainSpec>,
        l1_block_number: u64,
    ) -> Result<IndexerEvent, IndexerError> {
        // create a database transaction so this operation is atomic
        let txn = database.tx().await?;

        // delete batch inputs and l1 messages
        let batches_removed = txn.delete_batches_gt(l1_block_number).await?;
        let deleted_messages = txn.delete_l1_messages_gt(l1_block_number).await?;

        // filter and sort the executed L1 messages
        let mut removed_executed_l1_messages: Vec<_> =
            deleted_messages.into_iter().filter(|x| x.l2_block_number.is_some()).collect();
        removed_executed_l1_messages
            .sort_by(|a, b| a.transaction.queue_index.cmp(&b.transaction.queue_index));

        // check if we need to reorg the L2 head and delete some L2 blocks
        let (queue_index, l2_head_block_info) =
            if let Some(msg) = removed_executed_l1_messages.first() {
                let l2_reorg_block_number = msg
                    .l2_block_number
                    .expect("we guarantee that this is Some(u64) due to the filter on line 130") -
                    1;
                let l2_block_info = txn
                    .get_l2_block_info_by_number(l2_reorg_block_number)
                    .await?
                    .ok_or(IndexerError::L2BlockNotFound(l2_reorg_block_number))?;
                txn.delete_l2_blocks_gt(l2_reorg_block_number).await?;
                (Some(msg.transaction.queue_index), Some(l2_block_info))
            } else {
                (None, None)
            };

        // check if we need to reorg the L2 safe block
        let l2_safe_block_info = if batches_removed > 0 {
            if let Some(x) = txn.get_latest_safe_l2_block().await? {
                Some(x)
            } else {
                Some(BlockInfo::new(0, chain_spec.genesis_hash()))
            }
        } else {
            None
        };

        // commit the transaction
        txn.commit().await?;

        Ok(IndexerEvent::UnwindIndexed {
            l1_block_number,
            queue_index,
            l2_head_block_info,
            l2_safe_block_info,
        })
    }

    /// Handles a finalized event by updating the indexer L1 finalized block and returning the new
    /// finalized L2 chain block.
    async fn handle_finalized(
        database: Arc<Database>,
        block_number: u64,
        l1_block_number: Arc<AtomicU64>,
        l2_block_number: Arc<AtomicU64>,
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
        l1_block_number.store(block_number, Ordering::Relaxed);

        Ok(IndexerEvent::FinalizedIndexed(block_number, finalized_block))
    }

    /// Handles an L1 message by inserting it into the database.
    async fn handle_l1_message(
        database: Arc<Database>,
        chain_spec: Arc<ChainSpec>,
        l1_message: TxL1Message,
        l1_block_number: u64,
        block_timestamp: u64,
    ) -> Result<IndexerEvent, IndexerError> {
        let event = IndexerEvent::L1MessageIndexed(l1_message.queue_index);

        let queue_hash = if chain_spec
            .scroll_fork_activation(ScrollHardfork::EuclidV2)
            .active_at_timestamp_or_number(block_timestamp, l1_block_number) &&
            l1_message.queue_index > 0
        {
            let index = l1_message.queue_index - 1;
            let prev_queue_hash = database
                .get_l1_message_by_index(index)
                .await?
                .map(|m| m.queue_hash)
                .ok_or(DatabaseError::L1MessageNotFound(index))?;

            let mut input = prev_queue_hash.unwrap_or_default().to_vec();
            input.append(&mut l1_message.tx_hash().to_vec());
            Some(keccak256(input) & L1_MESSAGE_QUEUE_HASH_MASK)
        } else {
            None
        };

        let l1_message = L1MessageEnvelope::new(l1_message, l1_block_number, None, queue_hash);
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
        l1_block_number: Arc<AtomicU64>,
        l2_block_number: Arc<AtomicU64>,
    ) -> Result<IndexerEvent, IndexerError> {
        // finalized the batch.
        database.finalize_batch(batch_hash, block_number).await?;

        // check if the block where the batch was finalized is finalized on L1.
        let mut finalized_block = None;
        let l1_block_number_value = l1_block_number.load(Ordering::Relaxed);
        if l1_block_number_value > block_number {
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
        l2_block_number: Arc<AtomicU64>,
    ) -> Result<Option<BlockInfo>, IndexerError> {
        let finalized_block = database.get_highest_block_for_batch(batch_hash).await?;

        // only return the block if the indexer hasn't seen it.
        // in which case also update the `l2_finalized_block_number` value.
        Ok(finalized_block.filter(|info| {
            let current_l2_block_number = l2_block_number.load(Ordering::Relaxed);
            if info.number > current_l2_block_number {
                l2_block_number.store(info.number, Ordering::Relaxed);
                true
            } else {
                false
            }
        }))
    }

    /// Returns a boolean indicating whether the indexer is idle, meaning it has no pending futures.
    pub fn is_idle(&self) -> bool {
        self.pending_futures.is_empty()
    }
}

impl<ChainSpec: ScrollHardforks + 'static> Stream for Indexer<ChainSpec> {
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
    use std::vec;

    use super::*;
    use alloy_primitives::{address, bytes, U256};

    use arbitrary::{Arbitrary, Unstructured};
    use futures::StreamExt;
    use rand::Rng;
    use reth_scroll_chainspec::{ScrollChainSpec, SCROLL_MAINNET};
    use rollup_node_primitives::BatchCommitData;
    use scroll_db::test_utils::setup_test_db;

    async fn setup_test_indexer() -> (Indexer<ScrollChainSpec>, Arc<Database>) {
        let db = Arc::new(setup_test_db().await);
        (Indexer::new(db.clone(), SCROLL_MAINNET.clone()), db)
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

        let message = TxL1Message {
            queue_index: i64::arbitrary(&mut u).unwrap().unsigned_abs(),
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let block_number = u64::arbitrary(&mut u).unwrap();
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: message.clone(),
            block_number,
            block_timestamp: 0,
        });

        let _ = indexer.next().await;

        let l1_message_result =
            db.get_l1_message_by_index(message.queue_index).await.unwrap().unwrap();
        let l1_message = L1MessageEnvelope::new(message, block_number, None, None);

        assert_eq!(l1_message, l1_message_result);
    }

    #[tokio::test]
    async fn test_l1_message_hash_queue() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // insert the previous L1 message in database.
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: TxL1Message { queue_index: 1062109, ..Default::default() },
            block_number: 1475588,
            block_timestamp: 1745305199,
        });
        let _ = indexer.next().await.unwrap().unwrap();

        // <https://sepolia.scrollscan.com/tx/0xd80cd61ac5d8665919da19128cc8c16d3647e1e2e278b931769e986d01c6b910>
        let message = TxL1Message {
            queue_index: 1062110,
            gas_limit: 168000,
            to: address!("Ba50f5340FB9F3Bd074bD638c9BE13eCB36E603d"),
            value: U256::ZERO,
            sender: address!("61d8d3E7F7c656493d1d76aAA1a836CEdfCBc27b"),
            input: bytes!("8ef1332e000000000000000000000000323522a8de3cddeddbb67094eecaebc2436d6996000000000000000000000000323522a8de3cddeddbb67094eecaebc2436d699600000000000000000000000000000000000000000000000000038d7ea4c6800000000000000000000000000000000000000000000000000000000000001034de00000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000"),
        };
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: message.clone(),
            block_number: 14755883,
            block_timestamp: 1745305200,
        });

        let _ = indexer.next().await.unwrap().unwrap();

        let l1_message_result =
            db.get_l1_message_by_index(message.queue_index).await.unwrap().unwrap();

        assert_eq!(
            b256!("5e48ae1092c7f912849b9935f4e66870d2034b24fb2016f506e6754900000000"),
            l1_message_result.queue_hash.unwrap()
        );
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
        let l1_message_block_1 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 1,
            l2_block_number: None,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let l1_message_block_20 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 20,
            l2_block_number: None,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let l1_message_block_30 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 30,
            l2_block_number: None,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };

        // Index L1 messages
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_1.clone().transaction,
            block_number: l1_message_block_1.clone().l1_block_number,
            block_timestamp: 0,
        });
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_20.clone().transaction,
            block_number: l1_message_block_20.clone().l1_block_number,
            block_timestamp: 0,
        });
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_30.clone().transaction,
            block_number: l1_message_block_30.clone().l1_block_number,
            block_timestamp: 0,
        });

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

    #[tokio::test]
    async fn test_handle_reorg_executed_l1_messages() {
        // Instantiate indexer and db
        let (mut indexer, _database) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 8192];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a 3 random batch inputs and set their block numbers
        let batch_commit_block_1 =
            BatchCommitData { block_number: 5, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_commit_block_20 =
            BatchCommitData { block_number: 10, ..Arbitrary::arbitrary(&mut u).unwrap() };

        // Index batch inputs
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_20.clone()));
        for _ in 0..2 {
            let _event = indexer.next().await.unwrap().unwrap();
        }

        let batch_1 = BatchInfo::new(batch_commit_block_1.index, batch_commit_block_1.hash);
        let batch_20 = BatchInfo::new(batch_commit_block_20.index, batch_commit_block_20.hash);

        const UNITS_FOR_TESTING: u64 = 20;
        const L1_MESSAGES_NOT_EXECUTED_COUNT: u64 = 7;
        let mut l1_messages = Vec::with_capacity(UNITS_FOR_TESTING as usize);
        for l1_message_queue_index in 0..UNITS_FOR_TESTING {
            let l1_message = L1MessageEnvelope {
                queue_hash: None,
                l1_block_number: l1_message_queue_index,
                l2_block_number: (UNITS_FOR_TESTING - l1_message_queue_index >
                    L1_MESSAGES_NOT_EXECUTED_COUNT)
                    .then_some(l1_message_queue_index),
                transaction: TxL1Message {
                    queue_index: l1_message_queue_index,
                    ..Arbitrary::arbitrary(&mut u).unwrap()
                },
            };
            indexer.handle_l1_notification(L1Notification::L1Message {
                message: l1_message.transaction.clone(),
                block_number: l1_message.l1_block_number,
                block_timestamp: 0,
            });
            indexer.next().await.unwrap().unwrap();
            l1_messages.push(l1_message);
        }

        let mut blocks = Vec::with_capacity(UNITS_FOR_TESTING as usize);
        for block_number in 0..UNITS_FOR_TESTING {
            let l2_block = L2BlockInfoWithL1Messages {
                block_info: BlockInfo {
                    number: block_number,
                    hash: Arbitrary::arbitrary(&mut u).unwrap(),
                },
                l1_messages: (UNITS_FOR_TESTING - block_number > L1_MESSAGES_NOT_EXECUTED_COUNT)
                    .then_some(vec![l1_messages[block_number as usize].transaction.tx_hash()])
                    .unwrap_or_default(),
            };
            let batch_info = if block_number < 5 {
                Some(batch_1)
            } else if block_number < 10 {
                Some(batch_20)
            } else {
                None
            };
            indexer.handle_block(l2_block.clone(), batch_info);
            indexer.next().await.unwrap().unwrap();
            blocks.push(l2_block);
        }

        // First we assert that we dont reorg the L2 or message queue hash for a higher block
        // than any of the L1 messages.
        indexer.handle_l1_notification(L1Notification::Reorg(17));
        let event = indexer.next().await.unwrap().unwrap();
        assert_eq!(
            event,
            IndexerEvent::UnwindIndexed {
                l1_block_number: 17,
                queue_index: None,
                l2_head_block_info: None,
                l2_safe_block_info: None
            }
        );

        // Reorg at block 17 which is one of the messages that has not been executed yet. No reorg
        // but we should ensure the L1 messages have been deleted.
        indexer.handle_l1_notification(L1Notification::Reorg(7));
        let event = indexer.next().await.unwrap().unwrap();

        assert_eq!(
            event,
            IndexerEvent::UnwindIndexed {
                l1_block_number: 7,
                queue_index: Some(8),
                l2_head_block_info: Some(blocks[7].block_info),
                l2_safe_block_info: Some(blocks[4].block_info)
            }
        );

        // Now reorg at block 5 which contains L1 messages that have been executed .
        indexer.handle_l1_notification(L1Notification::Reorg(3));
        let event = indexer.next().await.unwrap().unwrap();

        assert_eq!(
            event,
            IndexerEvent::UnwindIndexed {
                l1_block_number: 3,
                queue_index: Some(4),
                l2_head_block_info: Some(blocks[3].block_info),
                l2_safe_block_info: Some(BlockInfo::new(0, indexer.chain_spec.genesis_hash())),
            }
        );
    }
}
