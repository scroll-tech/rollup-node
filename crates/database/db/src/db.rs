use super::{transaction::DatabaseTransaction, DatabaseConnectionProvider};
use crate::error::DatabaseError;

use sea_orm::{Database as SeaOrmDatabase, DatabaseConnection, TransactionTrait};

/// The [`Database`] struct is responsible for interacting with the database.
///
/// The [`Database`] type wraps a [`sea_orm::DatabaseConnection`]. We implement
/// [`DatabaseConnectionProvider`] for [`Database`] such that it can be used to perform the
/// operations defined in [`crate::DatabaseOperations`]. Atomic operations can be performed using
/// the [`Database::tx`] method which returns a [`DatabaseTransaction`] that also implements the
/// [`DatabaseConnectionProvider`] trait and also the [`crate::DatabaseOperations`] trait.
#[derive(Debug)]
pub struct Database {
    /// The underlying database connection.
    connection: DatabaseConnection,
}

impl Database {
    /// Creates a new [`Database`] instance associated with the provided database URL.
    pub async fn new(database_url: &str) -> Result<Self, DatabaseError> {
        let connection = SeaOrmDatabase::connect(database_url).await?;
        Ok(Self { connection })
    }

    /// Creates a new [`DatabaseTransaction`] which can be used for atomic operations.
    pub async fn tx(&self) -> Result<DatabaseTransaction, DatabaseError> {
        Ok(DatabaseTransaction::new(self.connection.begin().await?))
    }
}

impl DatabaseConnectionProvider for Database {
    type Connection = DatabaseConnection;

    fn get_connection(&self) -> &Self::Connection {
        &self.connection
    }
}

impl From<DatabaseConnection> for Database {
    fn from(connection: DatabaseConnection) -> Self {
        Self { connection }
    }
}

#[cfg(test)]
mod test {
    use crate::{
        models, operations::DatabaseOperations, test_utils::setup_test_db,
        DatabaseConnectionProvider,
    };
    use alloy_primitives::B256;
    use std::sync::Arc;

    use arbitrary::{Arbitrary, Unstructured};
    use futures::StreamExt;
    use rand::Rng;
    use rollup_node_primitives::{
        BatchCommitData, BatchInfo, BlockInfo, L1MessageEnvelope, L2BlockInfoWithL1Messages,
    };
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    #[tokio::test]
    async fn test_database_round_trip_batch_commit() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a random BatchCommitData.
        let batch_commit = BatchCommitData::arbitrary(&mut u).unwrap();

        // Round trip the BatchCommitData through the database.
        db.insert_batch(batch_commit.clone()).await.unwrap();
        let batch_commit_from_db =
            db.get_batch_by_index(batch_commit.index).await.unwrap().unwrap();
        assert_eq!(batch_commit, batch_commit_from_db);
    }

    #[tokio::test]
    async fn test_database_finalize_batch_commit() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a random BatchCommitData.
        let batch_commit = BatchCommitData::arbitrary(&mut u).unwrap();

        // Store the batch and finalize it.
        let finalized_block_number = u64::arbitrary(&mut u).unwrap();
        db.insert_batch(batch_commit.clone()).await.unwrap();
        db.finalize_batch(batch_commit.hash, finalized_block_number).await.unwrap();

        // Verify the finalized_block_number is correctly updated.
        let finalized_block_number_from_db = models::batch_commit::Entity::find()
            .filter(models::batch_commit::Column::Hash.eq(batch_commit.hash.to_vec()))
            .one(db.get_connection())
            .await
            .unwrap()
            .unwrap()
            .finalized_block_number
            .unwrap();
        assert_eq!(finalized_block_number, finalized_block_number_from_db as u64);
    }

    #[tokio::test]
    async fn test_database_round_trip_l1_message() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a random L1Message.
        let l1_message = L1MessageEnvelope::arbitrary(&mut u).unwrap();

        // Round trip the L1Message through the database.
        db.insert_l1_message(l1_message.clone()).await.unwrap();
        let l1_message_from_db_index =
            db.get_l1_message_by_index(l1_message.transaction.queue_index).await.unwrap().unwrap();
        let l1_message_from_db_hash =
            db.get_l1_message_by_hash(l1_message.queue_hash.unwrap()).await.unwrap().unwrap();
        assert_eq!(l1_message, l1_message_from_db_index);
        assert_eq!(l1_message, l1_message_from_db_hash);
    }

    #[tokio::test]
    #[ignore]
    async fn test_database_block_data_seed() {
        // Setup the test database.
        let db = setup_test_db().await;

        // db should contain the seeded data after migration.
        let data = db.get_l2_block_data_hint(0).await.unwrap();
        assert!(data.is_some());
    }

    #[tokio::test]
    async fn test_derived_block_exists() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate randoms BatchInfo and BlockInfo with increasing block numbers.
        let mut block_number = 100;
        let data = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = data.clone().into();
        db.insert_batch(data).await.unwrap();

        for _ in 0..10 {
            let block_info = L2BlockInfoWithL1Messages {
                block_info: BlockInfo {
                    number: block_number,
                    hash: B256::arbitrary(&mut u).unwrap(),
                },
                l1_messages: vec![],
            };
            db.insert_block(block_info, batch_info.into()).await.unwrap();
            block_number += 1;
        }

        // Fetch the highest block for the batch hash and verify number.
        let highest_block_info =
            db.get_highest_block_for_batch_hash(batch_info.hash).await.unwrap().unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);

        // Fetch the highest block for the batch and verify number.
        let highest_block_info =
            db.get_highest_block_for_batch_index(batch_info.index).await.unwrap().unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);
    }

    #[tokio::test]
    async fn test_derived_block_missing() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate randoms BatchInfo and BlockInfo with increasing block numbers.
        let mut block_number = 100;
        let first_batch = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let first_batch_info: BatchInfo = first_batch.clone().into();

        let second_batch = BatchCommitData { index: 250, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let second_batch_info: BatchInfo = second_batch.clone().into();

        db.insert_batch(first_batch).await.unwrap();
        db.insert_batch(second_batch).await.unwrap();

        for _ in 0..10 {
            let block_info = L2BlockInfoWithL1Messages {
                block_info: BlockInfo {
                    number: block_number,
                    hash: B256::arbitrary(&mut u).unwrap(),
                },
                l1_messages: vec![],
            };
            db.insert_block(block_info, first_batch_info.into()).await.unwrap();
            block_number += 1;
        }

        // Fetch the highest block for the batch hash and verify number.
        let highest_block_info =
            db.get_highest_block_for_batch_hash(second_batch_info.hash).await.unwrap().unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);

        // Fetch the highest block for the batch index and verify number.
        let highest_block_info =
            db.get_highest_block_for_batch_index(second_batch_info.index).await.unwrap().unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);
    }

    #[tokio::test]
    async fn test_database_finalized_batch_hash_at_height() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 2048];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate randoms BatchInfoCommitData, insert in database and finalize.
        let mut block_number = 100;
        let mut batch_index = 100;
        let mut highest_finalized_batch_hash = B256::ZERO;

        for _ in 0..20 {
            let data = BatchCommitData {
                index: batch_index,
                calldata: Arc::new(vec![].into()),
                ..Arbitrary::arbitrary(&mut u).unwrap()
            };
            let hash = data.hash;
            db.insert_batch(data).await.unwrap();

            // save batch hash finalized at block number 109.
            if block_number == 109 {
                highest_finalized_batch_hash = hash;
            }

            // Finalize batch up to block number 110.
            if block_number <= 110 {
                db.finalize_batch(hash, block_number).await.unwrap();
            }

            block_number += 1;
            batch_index += 1;
        }

        // Fetch the finalized batch for provided height and verify number.
        let highest_batch_hash_from_db =
            db.get_finalized_batch_hash_at_height(109).await.unwrap().unwrap();
        assert_eq!(highest_finalized_batch_hash, highest_batch_hash_from_db);
    }

    #[tokio::test]
    async fn test_database_tx() {
        // Setup the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 2048];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate 2 random L1Messages.
        let l1_message_1 = L1MessageEnvelope::arbitrary(&mut u).unwrap();
        let l1_message_2 = L1MessageEnvelope::arbitrary(&mut u).unwrap();

        // Insert the L1Messages into the database in a transaction.
        let tx = db.tx().await.unwrap();
        tx.insert_l1_message(l1_message_1.clone()).await.unwrap();
        tx.insert_l1_message(l1_message_2.clone()).await.unwrap();
        tx.commit().await.unwrap();

        // Check that the L1Messages are in the database.
        let l1_message_1_from_db = db
            .get_l1_message_by_index(l1_message_1.transaction.queue_index)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(l1_message_1, l1_message_1_from_db);
        let l1_message_2_from_db = db
            .get_l1_message_by_index(l1_message_2.transaction.queue_index)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(l1_message_2, l1_message_2_from_db);
    }

    #[tokio::test]
    async fn test_database_iterator() {
        // Setup the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 2048];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate 2 random L1Messages.
        let l1_message_1 = L1MessageEnvelope::arbitrary(&mut u).unwrap();
        let l1_message_2 = L1MessageEnvelope::arbitrary(&mut u).unwrap();

        // Insert the L1Messages into the database.
        db.insert_l1_message(l1_message_1.clone()).await.unwrap();
        db.insert_l1_message(l1_message_2.clone()).await.unwrap();

        // collect the L1Messages
        let l1_messages =
            db.get_l1_messages().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;

        // Apply the assertions.
        assert!(l1_messages.contains(&l1_message_1));
        assert!(l1_messages.contains(&l1_message_2));
    }

    #[tokio::test]
    async fn test_delete_l1_messages_gt() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate L1 messages with different L1 block numbers and queue indices
        for i in 0..10 {
            let mut l1_message = L1MessageEnvelope::arbitrary(&mut u).unwrap();
            l1_message.l1_block_number = 100 + i; // block number: 100-109
            l1_message.transaction.queue_index = i; // queue index: 0-9
            db.insert_l1_message(l1_message).await.unwrap();
        }

        // Delete messages with L1 block number > 105
        let deleted_messages = db.delete_l1_messages_gt(105).await.unwrap();

        // Verify that 4 messages were deleted (block numbers 106, 107, 108, 109)
        assert_eq!(deleted_messages.len(), 4);

        // Verify deleted messages have correct L1 block numbers
        for msg in &deleted_messages {
            assert!(msg.l1_block_number > 105);
        }

        // Verify remaining messages are still in database (queue indices 0-5)
        for queue_idx in 0..=5 {
            let msg = db.get_l1_message_by_index(queue_idx).await.unwrap();
            assert!(msg.is_some());
            assert!(msg.unwrap().l1_block_number <= 105);
        }

        // Verify deleted messages are no longer in database (queue indices 6-9)
        for queue_idx in 6..10 {
            let msg = db.get_l1_message_by_index(queue_idx).await.unwrap();
            assert!(msg.is_none());
        }
    }

    #[tokio::test]
    async fn test_get_l2_block_info_by_number() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate and insert a batch
        let batch_data = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = batch_data.clone().into();
        db.insert_batch(batch_data).await.unwrap();

        // Generate and insert multiple L2 blocks
        let mut block_infos = Vec::new();
        for i in 200..205 {
            let block_info = BlockInfo { number: i, hash: B256::arbitrary(&mut u).unwrap() };
            let l2_block = L2BlockInfoWithL1Messages { block_info, l1_messages: vec![] };
            block_infos.push(block_info);
            db.insert_block(l2_block, Some(batch_info)).await.unwrap();
        }

        // Test getting existing blocks
        for expected_block in block_infos {
            let retrieved_block =
                db.get_l2_block_info_by_number(expected_block.number).await.unwrap();
            assert_eq!(retrieved_block, Some(expected_block))
        }

        // Test getting non-existent block
        let non_existent_block = db.get_l2_block_info_by_number(999).await.unwrap();
        assert!(non_existent_block.is_none());
    }

    #[tokio::test]
    async fn test_get_latest_safe_l2_block() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Initially should return None
        let latest_safe = db.get_latest_safe_l2_info().await.unwrap();
        assert!(latest_safe.is_none());

        // Generate and insert a batch
        let batch_data = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = batch_data.clone().into();
        db.insert_batch(batch_data).await.unwrap();

        // Insert blocks with batch info (safe blocks)
        let safe_block_1 = BlockInfo { number: 200, hash: B256::arbitrary(&mut u).unwrap() };
        let safe_block_2 = BlockInfo { number: 201, hash: B256::arbitrary(&mut u).unwrap() };

        db.insert_block(
            L2BlockInfoWithL1Messages { block_info: safe_block_1, l1_messages: vec![] },
            Some(batch_info),
        )
        .await
        .unwrap();

        db.insert_block(
            L2BlockInfoWithL1Messages { block_info: safe_block_2, l1_messages: vec![] },
            Some(batch_info),
        )
        .await
        .unwrap();

        // Insert block without batch info (unsafe block)
        let unsafe_block = BlockInfo { number: 202, hash: B256::arbitrary(&mut u).unwrap() };
        db.insert_block(
            L2BlockInfoWithL1Messages { block_info: unsafe_block, l1_messages: vec![] },
            None,
        )
        .await
        .unwrap();

        // Should return the highest safe block (block 201)
        let latest_safe = db.get_latest_safe_l2_info().await.unwrap();
        assert_eq!(latest_safe, Some((safe_block_2, batch_info)));
    }

    #[tokio::test]
    async fn test_get_latest_l2_block() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Insert multiple blocks with increasing block numbers
        let mut latest_block = BlockInfo { number: 0, hash: B256::ZERO };
        for i in 300..305 {
            let block_info = BlockInfo { number: i, hash: B256::arbitrary(&mut u).unwrap() };
            latest_block = block_info;

            db.insert_block(L2BlockInfoWithL1Messages { block_info, l1_messages: vec![] }, None)
                .await
                .unwrap();
        }

        // Should return the block with highest number
        let retrieved_latest = db.get_latest_l2_block().await.unwrap();
        assert_eq!(retrieved_latest, Some(latest_block));
    }

    #[tokio::test]
    async fn test_delete_l2_blocks_gt_block_number() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Insert multiple L2 blocks
        for i in 400..410 {
            let block_info = BlockInfo { number: i, hash: B256::arbitrary(&mut u).unwrap() };

            db.insert_block(L2BlockInfoWithL1Messages { block_info, l1_messages: vec![] }, None)
                .await
                .unwrap();
        }

        // Delete blocks with number > 405
        let deleted_count = db.delete_l2_blocks_gt_block_number(405).await.unwrap();
        assert_eq!(deleted_count, 4); // Blocks 406, 407, 408, 409

        // Verify remaining blocks still exist
        for i in 400..=405 {
            let block = db.get_l2_block_info_by_number(i).await.unwrap();
            assert!(block.is_some());
        }

        // Verify deleted blocks no longer exist
        for i in 406..410 {
            let block = db.get_l2_block_info_by_number(i).await.unwrap();
            assert!(block.is_none());
        }
    }

    #[tokio::test]
    async fn test_delete_l2_blocks_gt_batch_index() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Insert multiple batches
        for i in 100..110 {
            let batch_data = BatchCommitData {
                index: i,
                calldata: Arc::new(vec![].into()),
                ..Arbitrary::arbitrary(&mut u).unwrap()
            };
            db.insert_batch(batch_data).await.unwrap();
        }

        // Insert L2 blocks with different batch indices
        for i in 100..110 {
            let batch_data = db.get_batch_by_index(i).await.unwrap().unwrap();
            let batch_info: BatchInfo = batch_data.into();

            let block_info = BlockInfo { number: 500 + i, hash: B256::arbitrary(&mut u).unwrap() };
            let l2_block = L2BlockInfoWithL1Messages { block_info, l1_messages: vec![] };

            db.insert_block(l2_block, Some(batch_info)).await.unwrap();
        }

        // Insert some blocks without batch index (should not be deleted)
        for i in 0..3 {
            let block_info = BlockInfo { number: 600 + i, hash: B256::arbitrary(&mut u).unwrap() };
            let l2_block = L2BlockInfoWithL1Messages { block_info, l1_messages: vec![] };

            db.insert_block(l2_block, None).await.unwrap();
        }

        // Delete L2 blocks with batch index > 105
        let deleted_count = db.delete_l2_blocks_gt_batch_index(105).await.unwrap();
        assert_eq!(deleted_count, 4); // Blocks with batch indices 106, 107, 108, 109

        // Verify remaining blocks with batch index <= 105 still exist
        for i in 100..=105 {
            let block = db.get_l2_block_info_by_number(500 + i).await.unwrap();
            assert!(block.is_some());
        }

        // Verify deleted blocks with batch index > 105 no longer exist
        for i in 106..110 {
            let block = db.get_l2_block_info_by_number(500 + i).await.unwrap();
            assert!(block.is_none());
        }

        // Verify blocks without batch index are still there (not affected by batch index filter)
        for i in 0..3 {
            let block = db.get_l2_block_info_by_number(600 + i).await.unwrap();
            assert!(block.is_some());
        }
    }

    #[tokio::test]
    async fn test_insert_block_with_l1_messages() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate and insert batch
        let batch_data = BatchCommitData { index: 10, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = batch_data.clone().into();
        db.insert_batch(batch_data).await.unwrap();

        // Generate and insert L1 messages
        let mut l1_message_hashes = Vec::new();
        for i in 100..103 {
            let mut l1_message = L1MessageEnvelope::arbitrary(&mut u).unwrap();
            l1_message.transaction.queue_index = i;
            l1_message_hashes.push(l1_message.transaction.tx_hash());
            db.insert_l1_message(l1_message).await.unwrap();
        }

        // Create block with L1 messages
        let block_info = BlockInfo { number: 500, hash: B256::arbitrary(&mut u).unwrap() };
        let l2_block =
            L2BlockInfoWithL1Messages { block_info, l1_messages: l1_message_hashes.clone() };

        // Insert block
        db.insert_block(l2_block, Some(batch_info)).await.unwrap();

        // Verify block was inserted
        let retrieved_block = db.get_l2_block_info_by_number(500).await.unwrap();
        assert_eq!(retrieved_block, Some(block_info));

        // Verify L1 messages were updated with L2 block number
        for i in 100..103 {
            let l1_message = db.get_l1_message_by_index(i).await.unwrap().unwrap();
            assert_eq!(l1_message.l2_block_number, Some(500));
        }
    }

    #[tokio::test]
    async fn test_insert_block_upsert_behavior() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate batches
        let batch_data_1 = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info_1: BatchInfo = batch_data_1.clone().into();
        let batch_data_2 = BatchCommitData { index: 200, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info_2: BatchInfo = batch_data_2.clone().into();

        db.insert_batch(batch_data_1).await.unwrap();
        db.insert_batch(batch_data_2).await.unwrap();

        // Insert initial block
        let block_info = BlockInfo { number: 600, hash: B256::arbitrary(&mut u).unwrap() };
        let l2_block = L2BlockInfoWithL1Messages { block_info, l1_messages: vec![] };

        db.insert_block(l2_block, Some(batch_info_1)).await.unwrap();

        // Verify initial insertion
        let retrieved_block = db.get_l2_block_info_by_number(600).await.unwrap();
        assert_eq!(retrieved_block, Some(block_info));

        // Verify initial batch association using model conversion
        let initial_l2_block_model = models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BlockNumber.eq(600))
            .one(db.get_connection())
            .await
            .unwrap()
            .unwrap();
        let (initial_block_info, initial_batch_info): (BlockInfo, Option<BatchInfo>) =
            initial_l2_block_model.into();
        assert_eq!(initial_block_info, block_info);
        assert_eq!(initial_batch_info, Some(batch_info_1));

        // Update the same block with different batch info (upsert)
        let updated_l2_block = L2BlockInfoWithL1Messages { block_info, l1_messages: vec![] };

        db.insert_block(updated_l2_block, Some(batch_info_2)).await.unwrap();

        // Verify the block still exists and was updated
        let retrieved_block = db.get_l2_block_info_by_number(600).await.unwrap().unwrap();
        assert_eq!(retrieved_block, block_info);

        // Verify batch association was updated using model conversion
        let updated_l2_block_model = models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BlockNumber.eq(600))
            .one(db.get_connection())
            .await
            .unwrap()
            .unwrap();
        let (updated_block_info, updated_batch_info): (BlockInfo, Option<BatchInfo>) =
            updated_l2_block_model.into();
        assert_eq!(updated_block_info, block_info);
        assert_eq!(updated_batch_info, Some(batch_info_2));
    }
}
