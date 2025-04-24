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
    use rollup_node_primitives::{BatchCommitData, BatchInfo, BlockInfo, L1MessageWithBlockNumber};
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
        let l1_message = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();

        // Round trip the L1Message through the database.
        db.insert_l1_message(l1_message.clone()).await.unwrap();
        let l1_message_from_db =
            db.get_l1_message(l1_message.transaction.queue_index).await.unwrap().unwrap();
        assert_eq!(l1_message, l1_message_from_db);
    }

    #[tokio::test]
    async fn test_database_block_data_seed() {
        // Setup the test database.
        let db = setup_test_db().await;

        // db should contain the seeded data after migration.
        let data = db.get_block_data(0.into()).await.unwrap();
        assert!(data.is_some());
    }

    #[tokio::test]
    async fn test_database_batch_to_block_exists() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate randoms BatchInfo and BlockInfo with increasing block numbers.
        let mut block_number = 100;
        let data = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info = data.clone().into();
        db.insert_batch(data).await.unwrap();

        for _ in 0..10 {
            let block_info =
                BlockInfo { number: block_number, hash: B256::arbitrary(&mut u).unwrap() };
            db.insert_batch_to_block(batch_info, block_info).await.unwrap();
            block_number += 1;
        }

        // Fetch the highest block for the batch and verify number.
        let highest_block_info =
            db.get_highest_block_for_batch(batch_info.hash).await.unwrap().unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);
    }

    #[tokio::test]
    async fn test_database_batch_to_block_missing() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate randoms BatchInfo and BlockInfo with increasing block numbers.
        let mut block_number = 100;
        let first_batch = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let first_batch_info = first_batch.clone().into();

        let second_batch = BatchCommitData { index: 250, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let second_batch_info: BatchInfo = second_batch.clone().into();

        db.insert_batch(first_batch).await.unwrap();
        db.insert_batch(second_batch).await.unwrap();

        for _ in 0..10 {
            let block_info =
                BlockInfo { number: block_number, hash: B256::arbitrary(&mut u).unwrap() };
            db.insert_batch_to_block(first_batch_info, block_info).await.unwrap();
            block_number += 1;
        }

        // Fetch the highest block for the batch and verify number.
        let highest_block_info =
            db.get_highest_block_for_batch(second_batch_info.hash).await.unwrap().unwrap();
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
        let l1_message_1 = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();
        let l1_message_2 = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();

        // Insert the L1Messages into the database in a transaction.
        let tx = db.tx().await.unwrap();
        tx.insert_l1_message(l1_message_1.clone()).await.unwrap();
        tx.insert_l1_message(l1_message_2.clone()).await.unwrap();
        tx.commit().await.unwrap();

        // Check that the L1Messages are in the database.
        let l1_message_1_from_db =
            db.get_l1_message(l1_message_1.transaction.queue_index).await.unwrap().unwrap();
        assert_eq!(l1_message_1, l1_message_1_from_db);
        let l1_message_2_from_db =
            db.get_l1_message(l1_message_2.transaction.queue_index).await.unwrap().unwrap();
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
        let l1_message_1 = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();
        let l1_message_2 = L1MessageWithBlockNumber::arbitrary(&mut u).unwrap();

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
}
