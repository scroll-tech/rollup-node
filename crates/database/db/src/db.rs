use std::{str::FromStr, time::Duration};

use super::transaction::{DatabaseTransactionProvider, TXMut, TX};
use crate::{error::DatabaseError, metrics::DatabaseMetrics, DatabaseConnectionProvider};

use sea_orm::{
    sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    DatabaseConnection, SqlxSqliteConnector, TransactionTrait,
};
use std::sync::Arc;
use tokio::sync::Mutex;

// TODO: make these configurable via CLI.

/// The timeout duration for database busy errors.
const BUSY_TIMEOUT_SECS: u64 = 5;

/// The maximum number of connections in the database connection pool.
const MAX_CONNECTIONS: u32 = 10;

/// The minimum number of connections in the database connection pool.
const MIN_CONNECTIONS: u32 = 5;

/// The timeout for acquiring a connection from the pool.
const ACQUIRE_TIMEOUT_SECS: u64 = 5;

/// The [`Database`] struct is responsible for interacting with the database.
///
/// The [`Database`] type hold a connection pool and a write lock. It implements the
/// [`DatabaseTransactionProvider`] trait to provide methods for creating read-only and read-write
/// transactions. It allows multiple concurrent read-only transactions, but ensures that only one
/// read-write transaction is active at any given time using a mutex.
#[derive(Debug)]
pub struct Database {
    /// The underlying database connection.
    connection: DatabaseConnection,
    /// A mutex to ensure that only one mutable transaction is active at a time.
    write_lock: Arc<Mutex<()>>,
    /// The database metrics.
    metrics: DatabaseMetrics,
    /// The temporary directory used for testing. We keep it here to ensure it lives as long as the
    /// database instance as the temp directory is deleted when [`tempfile::TempDir`] is dropped.
    #[cfg(feature = "test-utils")]
    tmp_dir: Option<tempfile::TempDir>,
}

impl Database {
    /// Creates a new [`Database`] instance associated with the provided database URL.
    pub async fn new(database_url: &str) -> Result<Self, DatabaseError> {
        Self::new_sqlite_with_pool_options(
            database_url,
            MAX_CONNECTIONS,
            MIN_CONNECTIONS,
            ACQUIRE_TIMEOUT_SECS,
            BUSY_TIMEOUT_SECS,
        )
        .await
    }

    /// Creates a new [`Database`] instance with SQLite-specific optimizations and custom pool
    /// settings.
    pub async fn new_sqlite_with_pool_options(
        database_url: &str,
        max_connections: u32,
        min_connections: u32,
        acquire_timeout_secs: u64,
        busy_timeout_secs: u64,
    ) -> Result<Self, DatabaseError> {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .journal_mode(sea_orm::sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(busy_timeout_secs))
            .foreign_keys(true)
            .synchronous(sea_orm::sqlx::sqlite::SqliteSynchronous::Normal);

        let sqlx_pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .min_connections(min_connections)
            .acquire_timeout(Duration::from_secs(acquire_timeout_secs))
            .connect_with(options)
            .await?;

        Ok(Self {
            connection: SqlxSqliteConnector::from_sqlx_sqlite_pool(sqlx_pool),
            write_lock: Arc::new(Mutex::new(())),
            metrics: DatabaseMetrics::default(),
            #[cfg(feature = "test-utils")]
            tmp_dir: None,
        })
    }

    /// Creates a new [`Database`] instance for testing purposes, using the provided temporary
    /// directory to store the database files.
    #[cfg(feature = "test-utils")]
    pub async fn test(dir: tempfile::TempDir) -> Result<Self, DatabaseError> {
        let path = dir.path().join("test.db");
        let mut db = Self::new(path.to_str().unwrap()).await?;
        db.tmp_dir = Some(dir);
        Ok(db)
    }

    /// Returns a reference to the database tmp dir.
    #[cfg(feature = "test-utils")]
    pub const fn tmp_dir(&self) -> Option<&tempfile::TempDir> {
        self.tmp_dir.as_ref()
    }
}

#[async_trait::async_trait]
impl DatabaseTransactionProvider for Database {
    /// Creates a new [`TX`] which can be used for read-only operations.
    async fn tx(&self) -> Result<TX, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Creating new read-only transaction");
        Ok(TX::new(self.connection.clone().begin().await?))
    }

    /// Creates a new [`TXMut`] which can be used for atomic read and write operations.
    async fn tx_mut(&self) -> Result<TXMut, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Creating new read-write transaction");
        let now = std::time::Instant::now();
        let guard = self.write_lock.clone().lock_owned().await;
        let tx_mut = TXMut::new(self.connection.clone().begin().await?, guard);
        let duration = now.elapsed().as_millis() as f64;
        self.metrics.lock_acquire_duration.record(duration);
        tracing::trace!(target: "scroll::db", "Acquired write lock in {} ms", duration);
        Ok(tx_mut)
    }
}

#[async_trait::async_trait]
impl DatabaseTransactionProvider for Arc<Database> {
    /// Creates a new [`TX`] which can be used for read-only operations.
    async fn tx(&self) -> Result<TX, DatabaseError> {
        self.as_ref().tx().await
    }

    /// Creates a new [`TXMut`] which can be used for atomic read and write operations.
    async fn tx_mut(&self) -> Result<TXMut, DatabaseError> {
        self.as_ref().tx_mut().await
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
        Self {
            connection,
            write_lock: Arc::new(Mutex::new(())),
            metrics: DatabaseMetrics::default(),
            #[cfg(feature = "test-utils")]
            tmp_dir: None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        models,
        operations::{DatabaseReadOperations, DatabaseWriteOperations},
        test_utils::setup_test_db,
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
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a random BatchCommitData.
        let batch_commit = BatchCommitData::arbitrary(&mut u).unwrap();

        // Round trip the BatchCommitData through the database.
        tx.insert_batch(batch_commit.clone()).await.unwrap();
        let batch_commit_from_db =
            tx.get_batch_by_index(batch_commit.index).await.unwrap().unwrap();

        tx.commit().await.unwrap();
        assert_eq!(batch_commit, batch_commit_from_db);
    }

    #[tokio::test]
    async fn test_database_finalize_batch_commits() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 2048];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate 10 finalized batches at L1 block 100.
        for i in 0..10 {
            let batch_commit = BatchCommitData {
                index: i,
                calldata: Arc::new(vec![].into()),
                finalized_block_number: Some(100),
                ..Arbitrary::arbitrary(&mut u).unwrap()
            };
            tx.insert_batch(batch_commit.clone()).await.unwrap();
        }
        // Finalize all batches below batch index 10.
        tx.finalize_batches_up_to_index(10, 100).await.unwrap();

        // Generate 10 commit batches not finalized.
        for i in 10..20 {
            let batch_commit = BatchCommitData {
                index: i,
                calldata: Arc::new(vec![].into()),
                finalized_block_number: None,
                ..Arbitrary::arbitrary(&mut u).unwrap()
            };
            tx.insert_batch(batch_commit.clone()).await.unwrap();
        }

        // Finalize all batches below batch index 15.
        tx.finalize_batches_up_to_index(15, 200).await.unwrap();

        // Verify the finalized_block_number is correctly updated.
        let batches = tx.get_batches().await.unwrap().collect::<Vec<Result<_, _>>>().await;
        for batch in batches {
            let batch = batch.unwrap();
            if batch.index < 10 {
                assert_eq!(batch.finalized_block_number, Some(100));
            } else if batch.index <= 15 {
                assert_eq!(batch.finalized_block_number, Some(200));
            } else {
                assert_eq!(batch.finalized_block_number, None);
            }
        }

        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn test_database_round_trip_l1_message() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a random L1Message.
        let l1_message = L1MessageEnvelope::arbitrary(&mut u).unwrap();

        // Round trip the L1Message through the database.
        tx.insert_l1_message(l1_message.clone()).await.unwrap();
        let l1_message_from_db_index =
            tx.get_l1_message_by_index(l1_message.transaction.queue_index).await.unwrap().unwrap();
        let l1_message_from_db_hash =
            tx.get_l1_message_by_hash(l1_message.queue_hash.unwrap()).await.unwrap().unwrap();

        tx.commit().await.unwrap();
        assert_eq!(l1_message, l1_message_from_db_index);
        assert_eq!(l1_message, l1_message_from_db_hash);
    }

    #[tokio::test]
    #[ignore]
    async fn test_database_block_data_seed() {
        // Setup the test database.
        let db = setup_test_db().await;
        let tx = db.tx().await.unwrap();

        // db should contain the seeded data after migration.
        let data = tx.get_l2_block_data_hint(0).await.unwrap();

        assert!(data.is_some());
    }

    #[tokio::test]
    async fn test_derived_block_exists() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate randoms BatchInfo and BlockInfo with increasing block numbers.
        let mut block_number = 100;
        let data = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = data.clone().into();
        tx.insert_batch(data).await.unwrap();

        for _ in 0..10 {
            let block_info =
                BlockInfo { number: block_number, hash: B256::arbitrary(&mut u).unwrap() };
            tx.insert_block(block_info, batch_info).await.unwrap();
            block_number += 1;
        }

        // Fetch the highest block for the batch hash and verify number.
        let highest_block_info =
            tx.get_highest_block_for_batch_hash(batch_info.hash).await.unwrap().unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);

        // Fetch the highest block for the batch and verify number.
        let highest_block_info =
            tx.get_highest_block_for_batch_index(batch_info.index).await.unwrap().unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);
    }

    #[tokio::test]
    async fn test_derived_block_missing() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

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

        tx.insert_batch(first_batch).await.unwrap();
        tx.insert_batch(second_batch).await.unwrap();

        for _ in 0..10 {
            let block_info =
                BlockInfo { number: block_number, hash: B256::arbitrary(&mut u).unwrap() };
            tx.insert_block(block_info, first_batch_info).await.unwrap();
            block_number += 1;
        }

        // Fetch the highest block for the batch hash and verify number.
        let highest_block_info =
            tx.get_highest_block_for_batch_hash(second_batch_info.hash).await.unwrap().unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);

        // Fetch the highest block for the batch index and verify number.
        let highest_block_info =
            tx.get_highest_block_for_batch_index(second_batch_info.index).await.unwrap().unwrap();

        tx.commit().await.unwrap();
        assert_eq!(highest_block_info.number, block_number - 1);
    }

    #[tokio::test]
    async fn test_database_batches_by_finalized_block_range() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 2048];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate randoms BatchInfoCommitData, insert in database and finalize.
        let mut block_number = 100;
        let mut batch_index = 100;
        let mut finalized_batches_hashes = vec![];

        for _ in 0..20 {
            let data = BatchCommitData {
                index: batch_index,
                calldata: Arc::new(vec![].into()),
                ..Arbitrary::arbitrary(&mut u).unwrap()
            };
            let hash = data.hash;
            tx.insert_batch(data).await.unwrap();

            // Finalize batch up to block number 110.
            if block_number <= 110 {
                finalized_batches_hashes.push(hash);
                tx.finalize_batches_up_to_index(batch_index, block_number).await.unwrap();
            }

            block_number += 1;
            batch_index += 1;
        }

        // Fetch the finalized batch for provided height and verify number.
        let batch_infos = tx
            .fetch_and_update_unprocessed_finalized_batches(110)
            .await
            .unwrap()
            .into_iter()
            .map(|b| b.hash)
            .collect::<Vec<_>>();
        assert_eq!(finalized_batches_hashes, batch_infos);
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
        let tx = db.tx_mut().await.unwrap();
        tx.insert_l1_message(l1_message_1.clone()).await.unwrap();
        tx.insert_l1_message(l1_message_2.clone()).await.unwrap();
        tx.commit().await.unwrap();

        // Check that the L1Messages are in the database.
        let tx = db.tx().await.unwrap();
        let l1_message_1_from_db = tx
            .get_l1_message_by_index(l1_message_1.transaction.queue_index)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(l1_message_1, l1_message_1_from_db);
        let l1_message_2_from_db = tx
            .get_l1_message_by_index(l1_message_2.transaction.queue_index)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(l1_message_2, l1_message_2_from_db);
    }

    #[tokio::test]
    async fn test_database_duplicate_l1_index() {
        // Setup the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 2048];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate 2 random L1Messages.
        let mut l1_message_1 = L1MessageEnvelope::arbitrary(&mut u).unwrap();
        let original_l1_message_1 = l1_message_1.clone();
        let l1_message_2 = L1MessageEnvelope::arbitrary(&mut u).unwrap();

        // Insert the L1Messages into the database in a transaction.
        let tx = db.tx_mut().await.unwrap();
        tx.insert_l1_message(l1_message_1.clone()).await.unwrap();
        tx.insert_l1_message(l1_message_2.clone()).await.unwrap();
        // Modify l1_block_number of l1_message_1 and attempt to insert again
        l1_message_1.l1_block_number = 1000;
        tx.insert_l1_message(l1_message_1.clone()).await.unwrap();
        tx.commit().await.unwrap();

        // Check that the L1Messages are in the database.
        let tx = db.tx().await.unwrap();
        let l1_message_1_from_db = tx
            .get_l1_message_by_index(l1_message_1.transaction.queue_index)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original_l1_message_1, l1_message_1_from_db);
        let l1_message_2_from_db = tx
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
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 2048];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate 2 random L1Messages.
        let l1_message_1 = L1MessageEnvelope::arbitrary(&mut u).unwrap();
        let l1_message_2 = L1MessageEnvelope::arbitrary(&mut u).unwrap();

        // Insert the L1Messages into the database.
        tx.insert_l1_message(l1_message_1.clone()).await.unwrap();
        tx.insert_l1_message(l1_message_2.clone()).await.unwrap();
        tx.commit().await.unwrap();

        // collect the L1Messages
        let tx = db.tx().await.unwrap();
        let l1_messages = tx
            .get_l1_messages(None)
            .await
            .unwrap()
            .unwrap()
            .map(|res| res.unwrap())
            .collect::<Vec<_>>()
            .await;

        // Apply the assertions.
        assert!(l1_messages.contains(&l1_message_1));
        assert!(l1_messages.contains(&l1_message_2));
    }

    #[tokio::test]
    async fn test_delete_l1_messages_gt() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate L1 messages with different L1 block numbers and queue indices
        for i in 0..10 {
            let mut l1_message = L1MessageEnvelope::arbitrary(&mut u).unwrap();
            l1_message.l1_block_number = 100 + i; // block number: 100-109
            l1_message.transaction.queue_index = i; // queue index: 0-9
            tx.insert_l1_message(l1_message).await.unwrap();
        }

        // Delete messages with L1 block number > 105
        let deleted_messages = tx.delete_l1_messages_gt(105).await.unwrap();

        // Verify that 4 messages were deleted (block numbers 106, 107, 108, 109)
        assert_eq!(deleted_messages.len(), 4);

        // Verify deleted messages have correct L1 block numbers
        for msg in &deleted_messages {
            assert!(msg.l1_block_number > 105);
        }

        // Verify remaining messages are still in database (queue indices 0-5)
        for queue_idx in 0..=5 {
            let msg = tx.get_l1_message_by_index(queue_idx).await.unwrap();
            assert!(msg.is_some());
            assert!(msg.unwrap().l1_block_number <= 105);
        }

        // Verify deleted messages are no longer in database (queue indices 6-9)
        for queue_idx in 6..10 {
            let msg = tx.get_l1_message_by_index(queue_idx).await.unwrap();
            assert!(msg.is_none());
        }
    }

    #[tokio::test]
    async fn test_get_l2_block_info_by_number() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate and insert a batch
        let batch_data = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = batch_data.clone().into();
        tx.insert_batch(batch_data).await.unwrap();

        // Generate and insert multiple L2 blocks
        let mut block_infos = Vec::new();
        for i in 200..205 {
            let block_info = BlockInfo { number: i, hash: B256::arbitrary(&mut u).unwrap() };
            let l2_block = block_info;
            block_infos.push(block_info);
            tx.insert_block(l2_block, batch_info).await.unwrap();
        }

        // Test getting existing blocks
        for expected_block in block_infos {
            let retrieved_block =
                tx.get_l2_block_info_by_number(expected_block.number).await.unwrap();
            assert_eq!(retrieved_block, Some(expected_block))
        }

        // Test getting non-existent block
        let non_existent_block = tx.get_l2_block_info_by_number(999).await.unwrap();
        assert!(non_existent_block.is_none());
    }

    #[tokio::test]
    async fn test_get_latest_safe_l2_block() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Initially should return the genesis block and hash.
        let (latest_safe_block, batch) = tx.get_latest_safe_l2_info().await.unwrap().unwrap();
        assert_eq!(latest_safe_block.number, 0);
        assert_eq!(batch.index, 0);

        // Generate and insert a batch
        let batch_data = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = batch_data.clone().into();
        tx.insert_batch(batch_data).await.unwrap();

        // Insert blocks with batch info (safe blocks)
        let safe_block_1 = BlockInfo { number: 200, hash: B256::arbitrary(&mut u).unwrap() };
        let safe_block_2 = BlockInfo { number: 201, hash: B256::arbitrary(&mut u).unwrap() };

        tx.insert_block(safe_block_1, batch_info).await.unwrap();

        tx.insert_block(safe_block_2, batch_info).await.unwrap();

        // Should return the highest safe block (block 201)
        let latest_safe = tx.get_latest_safe_l2_info().await.unwrap();
        assert_eq!(latest_safe, Some((safe_block_2, batch_info)));
    }

    #[tokio::test]
    async fn test_delete_l2_blocks_gt_block_number() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Insert multiple L2 blocks with batch info
        let batch_info = BatchInfo { index: 0, hash: B256::default() };
        for i in 400..410 {
            let block_info = BlockInfo { number: i, hash: B256::arbitrary(&mut u).unwrap() };

            tx.insert_block(block_info, batch_info).await.unwrap();
        }

        // Delete blocks with number > 405
        let deleted_count = tx.delete_l2_blocks_gt_block_number(405).await.unwrap();
        assert_eq!(deleted_count, 4); // Blocks 406, 407, 408, 409

        // Verify remaining blocks still exist
        for i in 400..=405 {
            let block = tx.get_l2_block_info_by_number(i).await.unwrap();
            assert!(block.is_some());
        }

        // Verify deleted blocks no longer exist
        for i in 406..410 {
            let block = tx.get_l2_block_info_by_number(i).await.unwrap();
            assert!(block.is_none());
        }
    }

    #[tokio::test]
    async fn test_delete_l2_blocks_gt_batch_index() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

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
            tx.insert_batch(batch_data).await.unwrap();
        }

        // Insert L2 blocks with different batch indices
        for i in 100..110 {
            let batch_data = tx.get_batch_by_index(i).await.unwrap().unwrap();
            let batch_info: BatchInfo = batch_data.into();

            let block_info = BlockInfo { number: 500 + i, hash: B256::arbitrary(&mut u).unwrap() };

            tx.insert_block(block_info, batch_info).await.unwrap();
        }

        // Delete L2 blocks with batch index > 105
        let deleted_count = tx.delete_l2_blocks_gt_batch_index(105).await.unwrap();
        assert_eq!(deleted_count, 4); // Blocks with batch indices 106, 107, 108, 109

        // Verify remaining blocks with batch index <= 105 still exist
        for i in 100..=105 {
            let block = tx.get_l2_block_info_by_number(500 + i).await.unwrap();
            assert!(block.is_some());
        }

        // Verify deleted blocks with batch index > 105 no longer exist
        for i in 106..110 {
            let block = tx.get_l2_block_info_by_number(500 + i).await.unwrap();
            assert!(block.is_none());
        }

        // Verify blocks without batch index are still there (not affected by batch index filter)
        for i in 0..3 {
            let block = tx.get_l2_block_info_by_number(600 + i).await.unwrap();
            assert!(block.is_some());
        }
    }

    #[tokio::test]
    async fn test_insert_block_with_l1_messages() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate and insert batch
        let batch_data = BatchCommitData { index: 10, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = batch_data.clone().into();
        tx.insert_batch(batch_data).await.unwrap();

        // Generate and insert L1 messages
        let mut l1_message_hashes = Vec::new();
        for i in 100..103 {
            let mut l1_message = L1MessageEnvelope::arbitrary(&mut u).unwrap();
            l1_message.transaction.queue_index = i;
            l1_message_hashes.push(l1_message.transaction.tx_hash());
            tx.insert_l1_message(l1_message).await.unwrap();
        }

        // Create block with L1 messages
        let block_info = BlockInfo { number: 500, hash: B256::arbitrary(&mut u).unwrap() };
        let l2_block =
            L2BlockInfoWithL1Messages { block_info, l1_messages: l1_message_hashes.clone() };

        // Insert block
        tx.insert_block(l2_block.block_info, batch_info).await.unwrap();
        tx.update_l1_messages_with_l2_block(l2_block).await.unwrap();

        // Verify block was inserted
        let retrieved_block = tx.get_l2_block_info_by_number(500).await.unwrap();
        assert_eq!(retrieved_block, Some(block_info));

        // Verify L1 messages were updated with L2 block number
        for i in 100..103 {
            let l1_message = tx.get_l1_message_by_index(i).await.unwrap().unwrap();
            assert_eq!(l1_message.l2_block_number, Some(500));
        }
    }

    #[tokio::test]
    async fn test_insert_block_upsert_behavior() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate batches
        let batch_data_1 = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info_1: BatchInfo = batch_data_1.clone().into();
        let batch_data_2 = BatchCommitData { index: 200, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info_2: BatchInfo = batch_data_2.clone().into();

        tx.insert_batch(batch_data_1).await.unwrap();
        tx.insert_batch(batch_data_2).await.unwrap();

        // Insert initial block
        let block_info = BlockInfo { number: 600, hash: B256::arbitrary(&mut u).unwrap() };
        tx.insert_block(block_info, batch_info_1).await.unwrap();

        // Verify initial insertion
        let retrieved_block = tx.get_l2_block_info_by_number(600).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(retrieved_block, Some(block_info));

        // Verify initial batch association using model conversion
        let initial_l2_block_model = models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BlockNumber.eq(600))
            .one(db.get_connection())
            .await
            .unwrap()
            .unwrap();
        let (initial_block_info, initial_batch_info): (BlockInfo, BatchInfo) =
            initial_l2_block_model.into();
        assert_eq!(initial_block_info, block_info);
        assert_eq!(initial_batch_info, batch_info_1);

        // Update the same block with different batch info (upsert)
        let tx = db.tx_mut().await.unwrap();
        tx.insert_block(block_info, batch_info_2).await.unwrap();
        tx.commit().await.unwrap();

        // Verify the block still exists and was updated
        let tx = db.tx().await.unwrap();
        let retrieved_block = tx.get_l2_block_info_by_number(600).await.unwrap().unwrap();
        drop(tx);
        assert_eq!(retrieved_block, block_info);

        // Verify batch association was updated using model conversion
        let updated_l2_block_model = models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BlockNumber.eq(600))
            .one(db.get_connection())
            .await
            .unwrap()
            .unwrap();
        let (updated_block_info, updated_batch_info): (BlockInfo, BatchInfo) =
            updated_l2_block_model.into();
        assert_eq!(updated_block_info, block_info);
        assert_eq!(updated_batch_info, batch_info_2);
    }

    #[tokio::test]
    async fn test_prepare_on_startup() {
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Insert batch 1 and associate it with two blocks in the database
        let batch_data_1 =
            BatchCommitData { index: 1, block_number: 10, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let block_1 = BlockInfo { number: 1, hash: B256::arbitrary(&mut u).unwrap() };
        let block_2 = BlockInfo { number: 2, hash: B256::arbitrary(&mut u).unwrap() };
        tx.insert_batch(batch_data_1.clone()).await.unwrap();
        tx.insert_block(block_1, batch_data_1.clone().into()).await.unwrap();
        tx.insert_block(block_2, batch_data_1.clone().into()).await.unwrap();

        // Insert batch 2 and associate it with one block in the database
        let batch_data_2 =
            BatchCommitData { index: 2, block_number: 20, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let block_3 = BlockInfo { number: 3, hash: B256::arbitrary(&mut u).unwrap() };
        tx.insert_batch(batch_data_2.clone()).await.unwrap();
        tx.insert_block(block_3, batch_data_2.clone().into()).await.unwrap();

        // Insert batch 3 produced at the same block number as batch 2 and associate it with one
        // block
        let batch_data_3 =
            BatchCommitData { index: 3, block_number: 20, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let block_4 = BlockInfo { number: 4, hash: B256::arbitrary(&mut u).unwrap() };
        tx.insert_batch(batch_data_3.clone()).await.unwrap();
        tx.insert_block(block_4, batch_data_3.clone().into()).await.unwrap();

        tx.set_finalized_l1_block_number(21).await.unwrap();
        tx.commit().await.unwrap();

        // Verify the batches and blocks were inserted correctly
        let tx = db.tx().await.unwrap();
        let retrieved_batch_1 = tx.get_batch_by_index(1).await.unwrap().unwrap();
        let retrieved_batch_2 = tx.get_batch_by_index(2).await.unwrap().unwrap();
        let retrieved_batch_3 = tx.get_batch_by_index(3).await.unwrap().unwrap();
        let retried_block_1 = tx.get_l2_block_info_by_number(1).await.unwrap().unwrap();
        let retried_block_2 = tx.get_l2_block_info_by_number(2).await.unwrap().unwrap();
        let retried_block_3 = tx.get_l2_block_info_by_number(3).await.unwrap().unwrap();
        let retried_block_4 = tx.get_l2_block_info_by_number(4).await.unwrap().unwrap();
        drop(tx);

        assert_eq!(retrieved_batch_1, batch_data_1);
        assert_eq!(retrieved_batch_2, batch_data_2);
        assert_eq!(retrieved_batch_3, batch_data_3);
        assert_eq!(retried_block_1, block_1);
        assert_eq!(retried_block_2, block_2);
        assert_eq!(retried_block_3, block_3);
        assert_eq!(retried_block_4, block_4);

        // Call prepare_on_startup which should not error
        let tx = db.tx_mut().await.unwrap();
        let result = tx.prepare_on_startup(Default::default()).await.unwrap();
        tx.commit().await.unwrap();

        // verify the result
        assert_eq!(result, (Some(block_2), Some(11)));

        // Verify that batches 2 and 3 are deleted
        let tx = db.tx().await.unwrap();
        let batch_1 = tx.get_batch_by_index(1).await.unwrap();
        let batch_2 = tx.get_batch_by_index(2).await.unwrap();
        let batch_3 = tx.get_batch_by_index(3).await.unwrap();
        assert!(batch_1.is_some());
        assert!(batch_2.is_none());
        assert!(batch_3.is_none());

        // Verify that blocks 3 and 4 are deleted
        let retried_block_3 = tx.get_l2_block_info_by_number(3).await.unwrap();
        let retried_block_4 = tx.get_l2_block_info_by_number(4).await.unwrap();
        assert!(retried_block_3.is_none());
        assert!(retried_block_4.is_none());
    }

    #[tokio::test]
    async fn test_l2_block_head_roundtrip() {
        // Set up the test database.
        let db = setup_test_db().await;
        let tx = db.tx_mut().await.unwrap();

        // Generate unstructured bytes.
        let mut bytes = [0u8; 40];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate and insert a block info as the head.
        let block_info = BlockInfo::arbitrary(&mut u).unwrap();
        tx.set_l2_head_block_number(block_info.number).await.unwrap();
        tx.commit().await.unwrap();

        // Retrieve and verify the head block info.
        let tx = db.tx().await.unwrap();
        let head_block_info = tx.get_l2_head_block_number().await.unwrap().unwrap();

        assert_eq!(head_block_info, block_info.number);
    }
}
