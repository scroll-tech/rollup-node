use super::transaction::{DatabaseTransactionProvider, TXMut, TX};
use crate::{
    error::DatabaseError,
    metrics::DatabaseMetrics,
    service::{query::DatabaseQuery, retry::Retry, DatabaseService, DatabaseServiceError},
    DatabaseConnectionProvider, DatabaseReadOperations, DatabaseWriteOperations, L1MessageKey,
    UnwindResult,
};
use alloy_primitives::{Signature, B256};
use rollup_node_primitives::{
    BatchCommitData, BatchConsolidationOutcome, BatchInfo, BlockInfo, L1MessageEnvelope,
    L2BlockInfoWithL1Messages,
};
use scroll_alloy_rpc_types_engine::BlockDataHint;
use sea_orm::{
    sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    DatabaseConnection, SqlxSqliteConnector, TransactionTrait,
};
use std::{fmt::Debug, future::Future, str::FromStr, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Semaphore};

// TODO: make these configurable via CLI.
/// The timeout duration for database busy errors.
const BUSY_TIMEOUT_SECS: u64 = 5;

/// The maximum number of connections in the database connection pool.
const MAX_CONNECTIONS: u32 = 32;

/// The minimum number of connections in the database connection pool.
const MIN_CONNECTIONS: u32 = 5;

/// The timeout for acquiring a connection from the pool.
const ACQUIRE_TIMEOUT_SECS: u64 = 5;

/// A wrapper around `DatabaseInner` which provides retry features.
#[derive(Debug)]
pub struct Database {
    database: Retry<Arc<DatabaseInner>>,
}

impl Database {
    /// Creates a new [`Database`] instance associated with the provided database URL.
    pub async fn new(database_url: &str) -> Result<Self, DatabaseError> {
        let db = Arc::new(DatabaseInner::new(database_url).await?);
        Ok(Self { database: Retry::new_with_default_config(db) })
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
        let db = Arc::new(
            DatabaseInner::new_sqlite_with_pool_options(
                database_url,
                max_connections,
                min_connections,
                acquire_timeout_secs,
                busy_timeout_secs,
            )
            .await?,
        );
        Ok(Self { database: Retry::new_with_default_config(db) })
    }

    /// Creates a new [`Database`] instance for testing purposes, using the provided temporary
    /// directory to store the database files.
    #[cfg(feature = "test-utils")]
    pub async fn test(dir: tempfile::TempDir) -> Result<Self, DatabaseError> {
        let db = Arc::new(DatabaseInner::test(dir).await?);
        Ok(Self { database: Retry::new_with_default_config(db) })
    }

    /// Returns a reference to the database tmp dir.
    #[cfg(feature = "test-utils")]
    pub fn tmp_dir(&self) -> Option<&tempfile::TempDir> {
        self.database.inner.tmp_dir()
    }

    /// Returns a reference to the inner database structure.
    pub fn inner(&self) -> Arc<DatabaseInner> {
        self.database.inner.clone()
    }

    /// Initiates a read operation to the underlying database layer.
    pub async fn tx<T: Send + 'static, Err: DatabaseServiceError, F, Fut>(
        &self,
        call: F,
    ) -> Result<T, Err>
    where
        F: Fn(Arc<TX>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Err>> + Send + 'static,
    {
        let request = DatabaseQuery::read(call);
        self.database.call(request).await
    }

    /// Initiates a write operation to the underlying database layer.
    pub async fn tx_mut<T: Send + 'static, Err: DatabaseServiceError, F, Fut>(
        &self,
        call: F,
    ) -> Result<T, Err>
    where
        F: Fn(Arc<TXMut>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Err>> + Send + 'static,
    {
        let request = DatabaseQuery::write(call);
        self.database.call(request).await
    }
}

#[async_trait::async_trait]
impl DatabaseWriteOperations for Database {
    async fn insert_batch(&self, batch_commit: BatchCommitData) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| {
            let batch_commit = batch_commit.clone();
            async move { tx.insert_batch(batch_commit).await }
        })
        .await
    }

    async fn finalize_batches_up_to_index(
        &self,
        batch_index: u64,
        block_number: u64,
    ) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move {
            tx.finalize_batches_up_to_index(batch_index, block_number).await
        })
        .await
    }

    async fn set_latest_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move { tx.set_latest_l1_block_number(block_number).await })
            .await
    }

    async fn set_finalized_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move { tx.set_finalized_l1_block_number(block_number).await })
            .await
    }

    async fn set_processed_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move { tx.set_processed_l1_block_number(block_number).await })
            .await
    }

    async fn set_l2_head_block_number(&self, number: u64) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move { tx.set_l2_head_block_number(number).await }).await
    }

    async fn fetch_and_update_unprocessed_finalized_batches(
        &self,
        finalized_l1_block_number: u64,
    ) -> Result<Vec<BatchInfo>, DatabaseError> {
        self.tx_mut(move |tx| async move {
            tx.fetch_and_update_unprocessed_finalized_batches(finalized_l1_block_number).await
        })
        .await
    }

    async fn delete_batches_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<u64, DatabaseError> {
        self.tx_mut(move |tx| async move { tx.delete_batches_gt_block_number(block_number).await })
            .await
    }

    async fn delete_batches_gt_batch_index(&self, batch_index: u64) -> Result<u64, DatabaseError> {
        self.tx_mut(move |tx| async move { tx.delete_batches_gt_batch_index(batch_index).await })
            .await
    }

    async fn insert_l1_message(&self, l1_message: L1MessageEnvelope) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| {
            let l1_message = l1_message.clone();
            async move { tx.insert_l1_message(l1_message).await }
        })
        .await
    }

    async fn update_skipped_l1_messages(&self, indexes: Vec<u64>) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| {
            let indexes = indexes.clone();
            async move { tx.update_skipped_l1_messages(indexes).await }
        })
        .await
    }

    async fn delete_l1_messages_gt(
        &self,
        l1_block_number: u64,
    ) -> Result<Vec<L1MessageEnvelope>, DatabaseError> {
        self.tx_mut(move |tx| async move { tx.delete_l1_messages_gt(l1_block_number).await }).await
    }

    async fn prepare_on_startup(
        &self,
        genesis_hash: B256,
    ) -> Result<(Option<BlockInfo>, Option<u64>), DatabaseError> {
        self.tx_mut(move |tx| async move { tx.prepare_on_startup(genesis_hash).await }).await
    }

    async fn delete_l2_blocks_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<u64, DatabaseError> {
        self.tx_mut(
            move |tx| async move { tx.delete_l2_blocks_gt_block_number(block_number).await },
        )
        .await
    }

    async fn delete_l2_blocks_gt_batch_index(
        &self,
        batch_index: u64,
    ) -> Result<u64, DatabaseError> {
        self.tx_mut(move |tx| async move { tx.delete_l2_blocks_gt_batch_index(batch_index).await })
            .await
    }

    async fn insert_blocks(
        &self,
        blocks: Vec<BlockInfo>,
        batch_info: BatchInfo,
    ) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| {
            let blocks = blocks.clone();
            async move { tx.insert_blocks(blocks, batch_info).await }
        })
        .await
    }

    async fn insert_block(
        &self,
        block_info: BlockInfo,
        batch_info: BatchInfo,
    ) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move { tx.insert_block(block_info, batch_info).await }).await
    }

    async fn insert_genesis_block(&self, genesis_hash: B256) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move { tx.insert_genesis_block(genesis_hash).await }).await
    }

    async fn update_l1_messages_from_l2_blocks(
        &self,
        blocks: Vec<L2BlockInfoWithL1Messages>,
    ) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| {
            let blocks = blocks.clone();
            async move { tx.update_l1_messages_from_l2_blocks(blocks).await }
        })
        .await
    }

    async fn update_l1_messages_with_l2_block(
        &self,
        block_info: L2BlockInfoWithL1Messages,
    ) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| {
            let block_info = block_info.clone();
            async move { tx.update_l1_messages_with_l2_block(block_info).await }
        })
        .await
    }

    async fn purge_l1_message_to_l2_block_mappings(
        &self,
        block_number: Option<u64>,
    ) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move {
            tx.purge_l1_message_to_l2_block_mappings(block_number).await
        })
        .await
    }

    async fn insert_batch_consolidation_outcome(
        &self,
        outcome: BatchConsolidationOutcome,
    ) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| {
            let outcome = outcome.clone();
            async move { tx.insert_batch_consolidation_outcome(outcome).await }
        })
        .await
    }

    async fn unwind(
        &self,
        genesis_hash: B256,
        l1_block_number: u64,
    ) -> Result<UnwindResult, DatabaseError> {
        self.tx_mut(move |tx| async move { tx.unwind(genesis_hash, l1_block_number).await }).await
    }

    async fn insert_signature(
        &self,
        block_hash: B256,
        signature: Signature,
    ) -> Result<(), DatabaseError> {
        self.tx_mut(move |tx| async move { tx.insert_signature(block_hash, signature).await }).await
    }
}

#[async_trait::async_trait]
impl DatabaseReadOperations for Database {
    async fn get_batch_by_index(
        &self,
        batch_index: u64,
    ) -> Result<Option<BatchCommitData>, DatabaseError> {
        self.tx(move |tx| async move { tx.get_batch_by_index(batch_index).await }).await
    }

    async fn get_latest_l1_block_number(&self) -> Result<u64, DatabaseError> {
        self.tx(|tx| async move { tx.get_latest_l1_block_number().await }).await
    }

    async fn get_finalized_l1_block_number(&self) -> Result<u64, DatabaseError> {
        self.tx(|tx| async move { tx.get_finalized_l1_block_number().await }).await
    }

    async fn get_processed_l1_block_number(&self) -> Result<u64, DatabaseError> {
        self.tx(|tx| async move { tx.get_processed_l1_block_number().await }).await
    }

    async fn get_l2_head_block_number(&self) -> Result<u64, DatabaseError> {
        self.tx(|tx| async move { tx.get_l2_head_block_number().await }).await
    }

    async fn get_n_l1_messages(
        &self,
        start: Option<L1MessageKey>,
        n: usize,
    ) -> Result<Vec<L1MessageEnvelope>, DatabaseError> {
        self.tx(move |tx| {
            let start = start.clone();
            async move { tx.get_n_l1_messages(start, n).await }
        })
        .await
    }

    async fn get_n_l2_block_data_hint(
        &self,
        block_number: u64,
        n: usize,
    ) -> Result<Vec<BlockDataHint>, DatabaseError> {
        self.tx(move |tx| async move { tx.get_n_l2_block_data_hint(block_number, n).await }).await
    }

    async fn get_l2_block_and_batch_info_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<(BlockInfo, BatchInfo)>, DatabaseError> {
        self.tx(move |tx| async move { tx.get_l2_block_and_batch_info_by_hash(block_hash).await })
            .await
    }

    async fn get_l2_block_info_by_number(
        &self,
        block_number: u64,
    ) -> Result<Option<BlockInfo>, DatabaseError> {
        self.tx(move |tx| async move { tx.get_l2_block_info_by_number(block_number).await }).await
    }

    async fn get_latest_safe_l2_info(
        &self,
    ) -> Result<Option<(BlockInfo, BatchInfo)>, DatabaseError> {
        self.tx(|tx| async move { tx.get_latest_safe_l2_info().await }).await
    }

    async fn get_highest_block_for_batch_hash(
        &self,
        batch_hash: B256,
    ) -> Result<Option<BlockInfo>, DatabaseError> {
        self.tx(move |tx| async move { tx.get_highest_block_for_batch_hash(batch_hash).await })
            .await
    }

    async fn get_highest_block_for_batch_index(
        &self,
        batch_index: u64,
    ) -> Result<Option<BlockInfo>, DatabaseError> {
        self.tx(move |tx| async move { tx.get_highest_block_for_batch_index(batch_index).await })
            .await
    }

    async fn get_signature(&self, block_hash: B256) -> Result<Option<Signature>, DatabaseError> {
        self.tx(move |tx| async move { tx.get_signature(block_hash).await }).await
    }
}

/// The [`DatabaseInner`] struct is responsible for interacting with the database.
///
/// The [`DatabaseInner`] type hold a connection pool and a write lock. It implements the
/// [`DatabaseTransactionProvider`] trait to provide methods for creating read-only and read-write
/// transactions. It allows multiple concurrent read-only transactions, but ensures that only one
/// read-write transaction is active at any given time using a mutex.
#[derive(Debug)]
pub struct DatabaseInner {
    /// The underlying database connection.
    connection: DatabaseConnection,
    /// A mutex to ensure that only one mutable transaction is active at a time.
    write_lock: Arc<Mutex<()>>,
    /// A semaphore to limit the number of concurrent read-only transactions.
    read_locks: Arc<Semaphore>,
    /// The database metrics.
    metrics: DatabaseMetrics,
    /// The temporary directory used for testing. We keep it here to ensure it lives as long as the
    /// database instance as the temp directory is deleted when [`tempfile::TempDir`] is dropped.
    #[cfg(feature = "test-utils")]
    tmp_dir: Option<tempfile::TempDir>,
}

impl DatabaseInner {
    /// Creates a new [`Database`] instance associated with the provided database URL.
    async fn new(database_url: &str) -> Result<Self, DatabaseError> {
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
    async fn new_sqlite_with_pool_options(
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

        // We reserve one connection for write transactions.
        let read_connection_limit = max_connections as usize - 1;

        Ok(Self {
            connection: SqlxSqliteConnector::from_sqlx_sqlite_pool(sqlx_pool),
            write_lock: Arc::new(Mutex::new(())),
            read_locks: Arc::new(Semaphore::new(read_connection_limit)),
            metrics: DatabaseMetrics::default(),
            #[cfg(feature = "test-utils")]
            tmp_dir: None,
        })
    }

    /// Creates a new [`Database`] instance for testing purposes, using the provided temporary
    /// directory to store the database files.
    #[cfg(feature = "test-utils")]
    async fn test(dir: tempfile::TempDir) -> Result<Self, DatabaseError> {
        let path = dir.path().join("test.db");
        let mut db = Self::new(path.to_str().unwrap()).await?;
        db.tmp_dir = Some(dir);
        Ok(db)
    }

    /// Returns a reference to the database tmp dir.
    #[cfg(feature = "test-utils")]
    const fn tmp_dir(&self) -> Option<&tempfile::TempDir> {
        self.tmp_dir.as_ref()
    }
}

#[async_trait::async_trait]
impl DatabaseTransactionProvider for DatabaseInner {
    /// Creates a new [`TX`] which can be used for read-only operations.
    async fn tx(&self) -> Result<TX, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Creating new read-only transaction");
        let permit = self.read_locks.clone().acquire_owned().await.unwrap();
        Ok(TX::new(self.connection.clone().begin().await?, permit))
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
impl DatabaseTransactionProvider for Arc<DatabaseInner> {
    /// Creates a new [`TX`] which can be used for read-only operations.
    async fn tx(&self) -> Result<TX, DatabaseError> {
        self.as_ref().tx().await
    }

    /// Creates a new [`TXMut`] which can be used for atomic read and write operations.
    async fn tx_mut(&self) -> Result<TXMut, DatabaseError> {
        self.as_ref().tx_mut().await
    }
}

impl DatabaseConnectionProvider for DatabaseInner {
    type Connection = DatabaseConnection;

    fn get_connection(&self) -> &Self::Connection {
        &self.connection
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
    use scroll_alloy_consensus::TxL1Message;
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
    async fn test_database_finalize_batch_commits() {
        // Set up the test database.
        let db = setup_test_db().await;

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
            db.insert_batch(batch_commit.clone()).await.unwrap();
        }
        // Finalize all batches below batch index 10.
        db.finalize_batches_up_to_index(10, 100).await.unwrap();

        // Generate 10 commit batches not finalized.
        for i in 10..20 {
            let batch_commit = BatchCommitData {
                index: i,
                calldata: Arc::new(vec![].into()),
                finalized_block_number: None,
                ..Arbitrary::arbitrary(&mut u).unwrap()
            };
            db.insert_batch(batch_commit.clone()).await.unwrap();
        }

        // Finalize all batches below batch index 15.
        db.finalize_batches_up_to_index(15, 200).await.unwrap();

        // Verify the finalized_block_number is correctly updated.
        let batches = models::batch_commit::Entity::find()
            .stream(db.inner().get_connection())
            .await
            .unwrap()
            .collect::<Vec<Result<_, _>>>()
            .await;
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
        let l1_message_from_db_index = db
            .get_n_l1_messages(
                Some(L1MessageKey::from_queue_index(l1_message.transaction.queue_index)),
                1,
            )
            .await
            .unwrap();
        let l1_message_from_db_hash = db
            .get_n_l1_messages(
                Some(L1MessageKey::from_queue_hash(l1_message.queue_hash.unwrap())),
                1,
            )
            .await
            .unwrap();

        assert_eq!(l1_message, l1_message_from_db_index[0]);
        assert_eq!(l1_message, l1_message_from_db_hash[0]);
    }

    #[tokio::test]
    #[ignore]
    async fn test_database_block_data_seed() {
        // Setup the test database.
        let db = setup_test_db().await;

        // db should contain the seeded data after migration.
        let data = db.get_n_l2_block_data_hint(0, 1).await.unwrap();

        assert_eq!(data.len(), 1);
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
            let block_info =
                BlockInfo { number: block_number, hash: B256::arbitrary(&mut u).unwrap() };
            db.insert_block(block_info, batch_info).await.unwrap();
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
            let block_info =
                BlockInfo { number: block_number, hash: B256::arbitrary(&mut u).unwrap() };
            db.insert_block(block_info, first_batch_info).await.unwrap();
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
    async fn test_database_batches_by_finalized_block_range() {
        // Set up the test database.
        let db = setup_test_db().await;

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
            db.insert_batch(data).await.unwrap();

            // Finalize batch up to block number 110.
            if block_number <= 110 {
                finalized_batches_hashes.push(hash);
                db.finalize_batches_up_to_index(batch_index, block_number).await.unwrap();
            }

            block_number += 1;
            batch_index += 1;
        }

        // Fetch the finalized batch for provided height and verify number.
        let batch_infos = db
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
        db.insert_l1_message(l1_message_1.clone()).await.unwrap();
        db.insert_l1_message(l1_message_2.clone()).await.unwrap();

        // Check that the L1Messages are in the database.
        let l1_message_1_from_db = db
            .get_n_l1_messages(
                Some(L1MessageKey::from_queue_index(l1_message_1.transaction.queue_index)),
                1,
            )
            .await
            .unwrap();
        assert_eq!(l1_message_1, l1_message_1_from_db[0]);
        let l1_message_2_from_db = db
            .get_n_l1_messages(
                Some(L1MessageKey::from_queue_index(l1_message_2.transaction.queue_index)),
                1,
            )
            .await
            .unwrap();

        assert_eq!(l1_message_2, l1_message_2_from_db[0]);
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
        db.insert_l1_message(l1_message_1.clone()).await.unwrap();
        db.insert_l1_message(l1_message_2.clone()).await.unwrap();
        // Modify l1_block_number of l1_message_1 and attempt to insert again
        l1_message_1.l1_block_number = 1000;
        db.insert_l1_message(l1_message_1.clone()).await.unwrap();

        // Check that the L1Messages are in the database.
        let l1_message_1_from_db = db
            .get_n_l1_messages(
                Some(L1MessageKey::from_queue_index(l1_message_1.transaction.queue_index)),
                1,
            )
            .await
            .unwrap();
        assert_eq!(original_l1_message_1, l1_message_1_from_db[0]);
        let l1_message_2_from_db = db
            .get_n_l1_messages(
                Some(L1MessageKey::from_queue_index(l1_message_2.transaction.queue_index)),
                1,
            )
            .await
            .unwrap();

        assert_eq!(l1_message_2, l1_message_2_from_db[0]);
    }

    #[tokio::test]
    async fn test_database_get_l1_messages() {
        // Setup the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 2048];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate 10 random L1Messages.
        let mut l1_messages = Vec::new();
        for i in 0..10 {
            let l1_message = L1MessageEnvelope {
                transaction: TxL1Message {
                    queue_index: i,
                    ..TxL1Message::arbitrary(&mut u).unwrap()
                },
                ..L1MessageEnvelope::arbitrary(&mut u).unwrap()
            };
            db.insert_l1_message(l1_message.clone()).await.unwrap();
            l1_messages.push(l1_message);
        }

        // collect the L1Messages
        let db_l1_messages = db.get_n_l1_messages(None, 5).await.unwrap();

        // Apply the assertions.
        assert_eq!(db_l1_messages, l1_messages[..5]);
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
            let msgs = db
                .get_n_l1_messages(Some(L1MessageKey::from_queue_index(queue_idx)), 1)
                .await
                .unwrap();
            assert!(!msgs.is_empty());
            assert!(msgs.first().unwrap().l1_block_number <= 105);
        }

        // Verify deleted messages are no longer in database (queue indices 6-9)
        for queue_idx in 6..10 {
            let msgs = db
                .get_n_l1_messages(Some(L1MessageKey::from_queue_index(queue_idx)), 1)
                .await
                .unwrap();
            assert!(msgs.is_empty());
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
            let l2_block = block_info;
            block_infos.push(block_info);
            db.insert_block(l2_block, batch_info).await.unwrap();
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

        // Initially should return the genesis block and hash.
        let (latest_safe_block, batch) = db.get_latest_safe_l2_info().await.unwrap().unwrap();
        assert_eq!(latest_safe_block.number, 0);
        assert_eq!(batch.index, 0);

        // Generate and insert a batch
        let batch_data = BatchCommitData { index: 100, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_info: BatchInfo = batch_data.clone().into();
        db.insert_batch(batch_data).await.unwrap();

        // Insert blocks with batch info (safe blocks)
        let safe_block_1 = BlockInfo { number: 200, hash: B256::arbitrary(&mut u).unwrap() };
        let safe_block_2 = BlockInfo { number: 201, hash: B256::arbitrary(&mut u).unwrap() };

        db.insert_block(safe_block_1, batch_info).await.unwrap();

        db.insert_block(safe_block_2, batch_info).await.unwrap();

        // Should return the highest safe block (block 201)
        let latest_safe = db.get_latest_safe_l2_info().await.unwrap();
        assert_eq!(latest_safe, Some((safe_block_2, batch_info)));
    }

    #[tokio::test]
    async fn test_delete_l2_blocks_gt_block_number() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Insert multiple L2 blocks with batch info
        let batch_info = BatchInfo { index: 0, hash: B256::default() };
        for i in 400..410 {
            let block_info = BlockInfo { number: i, hash: B256::arbitrary(&mut u).unwrap() };

            db.insert_block(block_info, batch_info).await.unwrap();
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

            db.insert_block(block_info, batch_info).await.unwrap();
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
        db.insert_block(l2_block.block_info, batch_info).await.unwrap();
        db.update_l1_messages_with_l2_block(l2_block).await.unwrap();

        // Verify block was inserted
        let retrieved_block = db.get_l2_block_info_by_number(500).await.unwrap();
        assert_eq!(retrieved_block, Some(block_info));

        // Verify L1 messages were updated with L2 block number
        for i in 100..103 {
            let l1_message =
                db.get_n_l1_messages(Some(L1MessageKey::from_queue_index(i)), 1).await.unwrap();
            assert_eq!(l1_message.first().unwrap().l2_block_number, Some(500));
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
        db.insert_block(block_info, batch_info_1).await.unwrap();

        // Verify initial insertion
        let retrieved_block = db.get_l2_block_info_by_number(600).await.unwrap();
        assert_eq!(retrieved_block, Some(block_info));

        // Verify initial batch association using model conversion
        let initial_l2_block_model = models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BlockNumber.eq(600))
            .one(db.inner().get_connection())
            .await
            .unwrap()
            .unwrap();
        let (initial_block_info, initial_batch_info): (BlockInfo, BatchInfo) =
            initial_l2_block_model.into();
        assert_eq!(initial_block_info, block_info);
        assert_eq!(initial_batch_info, batch_info_1);

        // Update the same block with different batch info (upsert)
        db.insert_block(block_info, batch_info_2).await.unwrap();

        // Verify the block still exists and was updated
        let retrieved_block = db.get_l2_block_info_by_number(600).await.unwrap().unwrap();
        assert_eq!(retrieved_block, block_info);

        // Verify batch association was updated using model conversion
        let updated_l2_block_model = models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BlockNumber.eq(600))
            .one(db.inner().get_connection())
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

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Insert batch 1 and associate it with two blocks in the database
        let batch_data_1 =
            BatchCommitData { index: 1, block_number: 10, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let block_1 = BlockInfo { number: 1, hash: B256::arbitrary(&mut u).unwrap() };
        let block_2 = BlockInfo { number: 2, hash: B256::arbitrary(&mut u).unwrap() };
        db.insert_batch(batch_data_1.clone()).await.unwrap();
        db.insert_block(block_1, batch_data_1.clone().into()).await.unwrap();
        db.insert_block(block_2, batch_data_1.clone().into()).await.unwrap();

        // Insert batch 2 and associate it with one block in the database
        let batch_data_2 =
            BatchCommitData { index: 2, block_number: 20, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let block_3 = BlockInfo { number: 3, hash: B256::arbitrary(&mut u).unwrap() };
        db.insert_batch(batch_data_2.clone()).await.unwrap();
        db.insert_block(block_3, batch_data_2.clone().into()).await.unwrap();

        // Insert batch 3 produced at the same block number as batch 2 and associate it with one
        // block
        let batch_data_3 =
            BatchCommitData { index: 3, block_number: 20, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let block_4 = BlockInfo { number: 4, hash: B256::arbitrary(&mut u).unwrap() };
        db.insert_batch(batch_data_3.clone()).await.unwrap();
        db.insert_block(block_4, batch_data_3.clone().into()).await.unwrap();

        db.set_finalized_l1_block_number(21).await.unwrap();

        // Verify the batches and blocks were inserted correctly
        let retrieved_batch_1 = db.get_batch_by_index(1).await.unwrap().unwrap();
        let retrieved_batch_2 = db.get_batch_by_index(2).await.unwrap().unwrap();
        let retrieved_batch_3 = db.get_batch_by_index(3).await.unwrap().unwrap();
        let retried_block_1 = db.get_l2_block_info_by_number(1).await.unwrap().unwrap();
        let retried_block_2 = db.get_l2_block_info_by_number(2).await.unwrap().unwrap();
        let retried_block_3 = db.get_l2_block_info_by_number(3).await.unwrap().unwrap();
        let retried_block_4 = db.get_l2_block_info_by_number(4).await.unwrap().unwrap();

        assert_eq!(retrieved_batch_1, batch_data_1);
        assert_eq!(retrieved_batch_2, batch_data_2);
        assert_eq!(retrieved_batch_3, batch_data_3);
        assert_eq!(retried_block_1, block_1);
        assert_eq!(retried_block_2, block_2);
        assert_eq!(retried_block_3, block_3);
        assert_eq!(retried_block_4, block_4);

        // Call prepare_on_startup which should not error
        let result = db.prepare_on_startup(Default::default()).await.unwrap();

        // verify the result
        assert_eq!(result, (Some(block_2), Some(11)));

        // Verify that batches 2 and 3 are deleted
        let batch_1 = db.get_batch_by_index(1).await.unwrap();
        let batch_2 = db.get_batch_by_index(2).await.unwrap();
        let batch_3 = db.get_batch_by_index(3).await.unwrap();
        assert!(batch_1.is_some());
        assert!(batch_2.is_none());
        assert!(batch_3.is_none());

        // Verify that blocks 3 and 4 are deleted
        let retried_block_3 = db.get_l2_block_info_by_number(3).await.unwrap();
        let retried_block_4 = db.get_l2_block_info_by_number(4).await.unwrap();
        assert!(retried_block_3.is_none());
        assert!(retried_block_4.is_none());
    }

    #[tokio::test]
    async fn test_l2_block_head_roundtrip() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 40];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate and insert a block info as the head.
        let block_info = BlockInfo::arbitrary(&mut u).unwrap();
        db.set_l2_head_block_number(block_info.number).await.unwrap();

        // Retrieve and verify the head block info.
        let head_block_info = db.get_l2_head_block_number().await.unwrap();

        assert_eq!(head_block_info, block_info.number);
    }
}
