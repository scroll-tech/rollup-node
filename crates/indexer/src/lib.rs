//! A library responsible for indexing data relevant to the L1.

use alloy_primitives::{b256, keccak256, B256};
use futures::StreamExt;
use reth_chainspec::EthChainSpec;
use rollup_node_primitives::{
    BatchCommitData, BatchInfo, BlockInfo, L1MessageEnvelope, L2BlockInfoWithL1Messages,
};
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_hardforks::{ScrollHardfork, ScrollHardforks};
use scroll_db::{Database, DatabaseError, DatabaseOperations, UnwindResult};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use strum::IntoEnumIterator;
use tokio_stream::wrappers::UnboundedReceiverStream;

mod event;
pub use event::IndexerEvent;

mod error;
pub use error::IndexerError;

mod handle;
pub use handle::{IndexerCommand, IndexerHandle};

mod metrics;
pub use metrics::{IndexerItem, IndexerMetrics};

const L1_MESSAGE_QUEUE_HASH_MASK: B256 =
    b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000");

/// The indexer is responsible for indexing data relevant to the L1.
#[derive(Debug)]
pub struct Indexer<ChainSpec> {
    /// A reference to the database used to persist the indexed data.
    database: Arc<Database>,
    /// The block number of the L1 finalized block.
    l1_finalized_block_number: Arc<AtomicU64>,
    /// The block number of the L2 finalized block.
    l2_finalized_block_number: Arc<AtomicU64>,
    /// The chain specification for the indexer.
    chain_spec: Arc<ChainSpec>,
    /// The metrics for the indexer.
    metrics: HashMap<IndexerItem, IndexerMetrics>,
    /// The receiver for indexer commands.
    command_rx: UnboundedReceiverStream<IndexerCommand>,
    /// The sender for indexer events.
    events_tx: tokio::sync::mpsc::UnboundedSender<Result<IndexerEvent, IndexerError>>,
}

impl<ChainSpec: ScrollHardforks + EthChainSpec + Send + Sync + 'static> Indexer<ChainSpec> {
    /// Creates a new indexer with the given [`Database`].
    fn new(database: Arc<Database>, chain_spec: Arc<ChainSpec>) -> (Self, IndexerHandle) {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        let indexer = Self {
            database,
            l1_finalized_block_number: Arc::new(AtomicU64::new(0)),
            l2_finalized_block_number: Arc::new(AtomicU64::new(0)),
            chain_spec,
            metrics: IndexerItem::iter()
                .map(|i| {
                    let label = i.as_str();
                    (i, IndexerMetrics::new_with_labels(&[("item", label)]))
                })
                .collect(),
            command_rx: command_rx.into(),
            events_tx,
        };
        let handle = IndexerHandle::new(command_tx, events_rx.into());
        (indexer, handle)
    }

    /// Spawns the indexer and returns a handle to it.
    pub fn spawn(database: Arc<Database>, chain_spec: Arc<ChainSpec>) -> IndexerHandle {
        let (indexer, handle) = Self::new(database, chain_spec);
        tokio::spawn(indexer.run());
        handle
    }

    /// Execution loop for the indexer.
    async fn run(mut self) {
        loop {
            while let Some(command) = self.command_rx.next().await {
                let maybe_result = match command {
                    IndexerCommand::L1Notification(notification) => {
                        self.handle_l1_notification(notification).await
                    }
                    IndexerCommand::Block { block, batch } => {
                        Some(self.handle_block(block, batch).await)
                    }
                };

                if let Some(result) = maybe_result {
                    if self.events_tx.send(result).is_err() {
                        tracing::warn!("Indexer event channel closed, stopping indexer.");
                        break;
                    }
                }
            }
        }
    }

    /// Handles an L2 block.
    async fn handle_block(
        &self,
        block_info: L2BlockInfoWithL1Messages,
        batch_info: Option<BatchInfo>,
    ) -> Result<IndexerEvent, IndexerError> {
        handle_metered!(self, IndexerItem::L2Block, async {
            self.database.insert_block(block_info.clone(), batch_info).await?;
            Result::<_, IndexerError>::Ok(IndexerEvent::BlockIndexed(block_info, batch_info))
        })
    }

    /// Handles an event from the L1.
    async fn handle_l1_notification(
        &self,
        event: L1Notification,
    ) -> Option<Result<IndexerEvent, IndexerError>> {
        match event {
            L1Notification::Reorg(block_number) => Some(handle_metered!(
                self,
                IndexerItem::L1Reorg,
                self.handle_l1_reorg(block_number)
            )),
            L1Notification::NewBlock(_) | L1Notification::Consensus(_) => None,
            L1Notification::Finalized(block_number) => Some(handle_metered!(
                self,
                IndexerItem::L1Finalization,
                self.handle_finalized(block_number)
            )),
            L1Notification::BatchCommit(batch) => Some(handle_metered!(
                self,
                IndexerItem::BatchCommit,
                self.handle_batch_commit(batch)
            )),
            L1Notification::L1Message { message, block_number, block_timestamp } => {
                Some(handle_metered!(
                    self,
                    IndexerItem::L1Message,
                    self.handle_l1_message(message, block_number, block_timestamp)
                ))
            }
            L1Notification::BatchFinalization { hash, block_number, .. } => Some(handle_metered!(
                self,
                IndexerItem::BatchFinalization,
                Box::pin(self.handle_batch_finalization(hash, block_number))
            )),
        }
    }

    /// Handles a reorganization event by deleting all indexed data which is greater than the
    /// provided block number.
    async fn handle_l1_reorg(&self, l1_block_number: u64) -> Result<IndexerEvent, IndexerError> {
        let txn = self.database.tx().await?;
        let UnwindResult { l1_block_number, queue_index, l2_head_block_info, l2_safe_block_info } =
            txn.unwind(self.chain_spec.genesis_hash(), l1_block_number).await?;
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
    async fn handle_finalized(&self, block_number: u64) -> Result<IndexerEvent, IndexerError> {
        // Set the latest finalized L1 block in the database.
        self.database.set_latest_finalized_l1_block_number(block_number).await?;

        // get the newest finalized batch.
        let batch_hash = self.database.get_finalized_batch_hash_at_height(block_number).await?;

        // get the finalized block for the batch.
        let finalized_block = if let Some(hash) = batch_hash {
            Self::fetch_highest_finalized_block(
                &self.database,
                hash,
                &self.l2_finalized_block_number,
            )
            .await?
        } else {
            None
        };

        // update the indexer l1 block number.
        self.l1_finalized_block_number.store(block_number, Ordering::Relaxed);

        Ok(IndexerEvent::FinalizedIndexed(block_number, finalized_block))
    }

    /// Handles an L1 message by inserting it into the database.
    async fn handle_l1_message(
        &self,
        l1_message: TxL1Message,
        l1_block_number: u64,
        block_timestamp: u64,
    ) -> Result<IndexerEvent, IndexerError> {
        let event = IndexerEvent::L1MessageIndexed(l1_message.queue_index);

        let queue_hash = if self
            .chain_spec
            .scroll_fork_activation(ScrollHardfork::EuclidV2)
            .active_at_timestamp_or_number(block_timestamp, l1_block_number) &&
            l1_message.queue_index > 0
        {
            let index = l1_message.queue_index - 1;
            let prev_queue_hash = self
                .database
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
        self.database.insert_l1_message(l1_message).await?;
        Ok(event)
    }

    /// Handles a batch input by inserting it into the database.
    async fn handle_batch_commit(
        &self,
        batch: BatchCommitData,
    ) -> Result<IndexerEvent, IndexerError> {
        let txn = self.database.tx().await?;
        let prev_batch_index = batch.index - 1;

        // remove any batches with an index greater than the previous batch.
        let affected = txn.delete_batches_gt_batch_index(prev_batch_index).await?;

        // handle the case of a batch revert.
        let new_safe_head = if affected > 0 {
            txn.delete_l2_blocks_gt_batch_index(prev_batch_index).await?;
            txn.get_highest_block_for_batch_index(prev_batch_index).await?
        } else {
            None
        };

        let event = IndexerEvent::BatchCommitIndexed {
            batch_info: BatchInfo::new(batch.index, batch.hash),
            safe_head: new_safe_head,
            l1_block_number: batch.block_number,
        };

        // insert the batch and commit the transaction.
        txn.insert_batch(batch).await?;
        txn.commit().await?;

        Ok(event)
    }

    /// Handles a batch finalization event by updating the batch input in the database.
    async fn handle_batch_finalization(
        &self,
        batch_hash: B256,
        block_number: u64,
    ) -> Result<IndexerEvent, IndexerError> {
        // finalized the batch.
        self.database.finalize_batch(batch_hash, block_number).await?;

        // check if the block where the batch was finalized is finalized on L1.
        let mut finalized_block = None;
        let l1_block_number_value = self.l1_finalized_block_number.load(Ordering::Relaxed);
        if l1_block_number_value > block_number {
            // fetch the finalized block.
            finalized_block = Self::fetch_highest_finalized_block(
                &self.database,
                batch_hash,
                &self.l2_finalized_block_number,
            )
            .await?;
        }

        let event = IndexerEvent::BatchFinalizationIndexed(batch_hash, finalized_block);
        Ok(event)
    }

    /// Returns the highest finalized block for the provided batch hash. Will return [`None`] if the
    /// block number has already been seen by the indexer.
    async fn fetch_highest_finalized_block(
        database: &Arc<Database>,
        batch_hash: B256,
        l2_block_number: &Arc<AtomicU64>,
    ) -> Result<Option<BlockInfo>, IndexerError> {
        let finalized_block = database.get_highest_block_for_batch_hash(batch_hash).await?;

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
}

/// Macro to meter the execution of an indexer operation.
#[macro_export]
macro_rules! handle_metered {
    ($self:ident, $item:expr, $fut:expr) => {{
        let metric = $self.metrics.get(&$item).expect("metric exists").clone();
        let now = std::time::Instant::now();
        let res = $fut.await;
        metric.task_duration.record(now.elapsed().as_secs_f64());
        res
    }};
}

#[cfg(test)]
mod test {
    use std::vec;

    use super::*;
    use alloy_primitives::{address, bytes, U256};

    use arbitrary::{Arbitrary, Unstructured};
    use futures::StreamExt;
    use rand::Rng;
    use reth_scroll_chainspec::SCROLL_MAINNET;
    use rollup_node_primitives::BatchCommitData;
    use scroll_db::test_utils::setup_test_db;

    async fn setup_test_indexer() -> (IndexerHandle, Arc<Database>) {
        let db = Arc::new(setup_test_db().await);
        let handle = Indexer::spawn(db.clone(), SCROLL_MAINNET.clone());
        (handle, db)
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
        let _ = indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit.clone()));

        let event = indexer.next().await.unwrap().unwrap();

        // Verify the event structure
        match event {
            IndexerEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, batch_commit.index);
                assert_eq!(batch_info.hash, batch_commit.hash);
                assert_eq!(safe_head, None); // No safe head since no batch revert
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        let batch_commit_result = db.get_batch_by_index(batch_commit.index).await.unwrap().unwrap();
        assert_eq!(batch_commit, batch_commit_result);
    }

    #[tokio::test]
    async fn test_handle_batch_commit_with_revert() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Create sequential batches
        let batch_1 = BatchCommitData {
            index: 100,
            calldata: Arc::new(vec![].into()),
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let batch_2 = BatchCommitData {
            index: 101,
            calldata: Arc::new(vec![].into()),
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let batch_3 = BatchCommitData {
            index: 102,
            calldata: Arc::new(vec![].into()),
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };

        // Index first batch
        let _ = indexer.handle_l1_notification(L1Notification::BatchCommit(batch_1.clone()));
        let event = indexer.next().await.unwrap().unwrap();
        match event {
            IndexerEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, 100);
                assert_eq!(safe_head, None);
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        // Index second batch
        let _ = indexer.handle_l1_notification(L1Notification::BatchCommit(batch_2.clone()));
        let event = indexer.next().await.unwrap().unwrap();
        match event {
            IndexerEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, 101);
                assert_eq!(safe_head, None);
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        // Index third batch
        let _ = indexer.handle_l1_notification(L1Notification::BatchCommit(batch_3.clone()));
        let event = indexer.next().await.unwrap().unwrap();
        match event {
            IndexerEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, 102);
                assert_eq!(safe_head, None);
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        // Add some L2 blocks for the batches
        let batch_1_info = BatchInfo::new(batch_1.index, batch_1.hash);
        let batch_2_info = BatchInfo::new(batch_2.index, batch_2.hash);

        let block_1 = L2BlockInfoWithL1Messages {
            block_info: BlockInfo { number: 500, hash: Arbitrary::arbitrary(&mut u).unwrap() },
            l1_messages: vec![],
        };
        let block_2 = L2BlockInfoWithL1Messages {
            block_info: BlockInfo { number: 501, hash: Arbitrary::arbitrary(&mut u).unwrap() },
            l1_messages: vec![],
        };
        let block_3 = L2BlockInfoWithL1Messages {
            block_info: BlockInfo { number: 502, hash: Arbitrary::arbitrary(&mut u).unwrap() },
            l1_messages: vec![],
        };

        let _ = indexer.handle_block(block_1.clone(), Some(batch_1_info));
        indexer.next().await.unwrap().unwrap();

        let _ = indexer.handle_block(block_2.clone(), Some(batch_2_info));
        indexer.next().await.unwrap().unwrap();

        let _ = indexer.handle_block(block_3.clone(), Some(batch_2_info));
        indexer.next().await.unwrap().unwrap();

        // Now simulate a batch revert by submitting a new batch with index 101
        // This should delete batch 102 and any blocks associated with it
        let new_batch_2 = BatchCommitData {
            index: 101,
            calldata: Arc::new(vec![1, 2, 3].into()), // Different data
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };

        let _ = indexer.handle_l1_notification(L1Notification::BatchCommit(new_batch_2.clone()));
        let event = indexer.next().await.unwrap().unwrap();

        // Verify the event indicates a batch revert
        match event {
            IndexerEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, 101);
                assert_eq!(batch_info.hash, new_batch_2.hash);
                // Safe head should be the highest block from batch index <= 100
                assert_eq!(safe_head, Some(block_1.block_info));
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        // Verify batch 102 was deleted
        let batch_102 = db.get_batch_by_index(102).await.unwrap();
        assert!(batch_102.is_none());

        // Verify batch 101 was replaced with new data
        let updated_batch_101 = db.get_batch_by_index(101).await.unwrap().unwrap();
        assert_eq!(updated_batch_101, new_batch_2);

        // Verify batch 100 still exists
        let batch_100 = db.get_batch_by_index(100).await.unwrap();
        assert!(batch_100.is_some());
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
        let _ = indexer.handle_l1_notification(L1Notification::L1Message {
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
        let _ = indexer.handle_l1_notification(L1Notification::L1Message {
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
        let _ = indexer.handle_l1_notification(L1Notification::L1Message {
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
        batch_commit_block_1.index = 1;
        let batch_commit_block_1 = batch_commit_block_1;

        let mut batch_commit_block_20 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_20.block_number = 20;
        batch_commit_block_20.index = 20;
        let batch_commit_block_20 = batch_commit_block_20;

        let mut batch_commit_block_30 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_30.block_number = 30;
        batch_commit_block_30.index = 30;
        let batch_commit_block_30 = batch_commit_block_30;

        // Index batch inputs
        let _ = indexer
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
        let _ = indexer
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_20.clone()));
        let _ = indexer
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_30.clone()));

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
        let _ = indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_1.clone().transaction,
            block_number: l1_message_block_1.clone().l1_block_number,
            block_timestamp: 0,
        });
        let _ = indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_20.clone().transaction,
            block_number: l1_message_block_20.clone().l1_block_number,
            block_timestamp: 0,
        });
        let _ = indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_30.clone().transaction,
            block_number: l1_message_block_30.clone().l1_block_number,
            block_timestamp: 0,
        });

        // Reorg at block 20
        let _ = indexer.handle_l1_notification(L1Notification::Reorg(20));

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
            BatchCommitData { block_number: 5, index: 5, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_commit_block_10 = BatchCommitData {
            block_number: 10,
            index: 10,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };

        // Index batch inputs
        let _ = indexer
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
        let _ = indexer
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_10.clone()));
        for _ in 0..2 {
            let _event = indexer.next().await.unwrap().unwrap();
        }

        let batch_1 = BatchInfo::new(batch_commit_block_1.index, batch_commit_block_1.hash);
        let batch_10 = BatchInfo::new(batch_commit_block_10.index, batch_commit_block_10.hash);

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
            let _ = indexer.handle_l1_notification(L1Notification::L1Message {
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
                Some(batch_10)
            } else {
                None
            };
            let _ = indexer.handle_block(l2_block.clone(), batch_info);
            indexer.next().await.unwrap().unwrap();
            blocks.push(l2_block);
        }

        // First we assert that we dont reorg the L2 or message queue hash for a higher block
        // than any of the L1 messages.
        let _ = indexer.handle_l1_notification(L1Notification::Reorg(17));
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

        // Reorg at block 7 which is one of the messages that has not been executed yet. No reorg
        // but we should ensure the L1 messages have been deleted.
        let _ = indexer.handle_l1_notification(L1Notification::Reorg(7));
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
        let _ = indexer.handle_l1_notification(L1Notification::Reorg(3));
        let event = indexer.next().await.unwrap().unwrap();

        assert_eq!(
            event,
            IndexerEvent::UnwindIndexed {
                l1_block_number: 3,
                queue_index: Some(4),
                l2_head_block_info: Some(blocks[3].block_info),
                l2_safe_block_info: Some(BlockInfo::new(0, SCROLL_MAINNET.genesis_hash())),
            }
        );
    }
}
