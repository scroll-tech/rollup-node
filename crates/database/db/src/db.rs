use crate::error::DatabaseError;

use super::models;
use alloy_primitives::B256;
use futures::{Stream, StreamExt};
use rollup_node_primitives::{BatchInput, L1MessageWithBlockNumber};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, Database as SeaOrmDatabase, DatabaseConnection,
    DatabaseTransaction, DbErr, EntityTrait, QueryFilter, Set, StreamTrait, TransactionTrait,
};

/// The [`Database`] struct is responsible for interacting with the database.
///
/// The [`Database`] type wraps a generic DB and provides methods for
/// interacting with the database. The DB abstracts the underlying
/// database connection and provides a high-level API for interacting with the database. We have
/// implemented support for [`Database<DatabaseConnection>`] and [`Database<DatabaseTransaction>`].
/// The [`Database`] struct provides methods for inserting and querying [`BatchInput`]s and
/// [`L1MessageWithBlockNumber`]s from the database. Atomic transaction support is provided by the
/// [`Database::tx`] method, which returns a [`Database`] instance with support for
/// [`Database::commit`] and [`Database::rollback`] that can be used to execute multiple database
/// operations in a single transaction.
#[derive(Debug)]
pub struct Database<DB> {
    /// The underlying database connection.
    pub(crate) connection: DB,
}

impl Database<DatabaseConnection> {
    /// Creates a new [`Database`] instance associated with the provided database URL.
    pub async fn new(database_url: &str) -> Result<Self, DatabaseError> {
        let connection = SeaOrmDatabase::connect(database_url).await?;
        Ok(Self { connection })
    }

    /// Creates a new [`DatabaseTransaction`] which can be used for atomic operations.
    pub async fn tx(&self) -> Result<Database<DatabaseTransaction>, DatabaseError> {
        Ok(Database::<DatabaseTransaction> { connection: self.connection.begin().await? })
    }
}

impl Database<DatabaseTransaction> {
    /// Commits the transaction.
    pub async fn commit(self) -> Result<(), DatabaseError> {
        self.connection.commit().await?;
        Ok(())
    }

    /// Rolls back the transaction.
    pub async fn rollback(self) -> Result<(), DatabaseError> {
        self.connection.rollback().await?;
        Ok(())
    }
}

impl<DB: ConnectionTrait + StreamTrait> Database<DB> {
    /// Insert a [`BatchInput`] into the database and returns the batch input model
    /// ([`models::batch_input::Model`]).
    pub async fn insert_batch_input(
        &self,
        batch_input: BatchInput,
    ) -> Result<models::batch_input::Model, DatabaseError> {
        tracing::trace!(target: "scroll::db", batch_hash = ?batch_input.batch_hash(), batch_index = batch_input.batch_index(), "Inserting batch input into database.");
        let batch_input: models::batch_input::ActiveModel = batch_input.into();
        Ok(batch_input.insert(&self.connection).await?)
    }

    /// Finalize a [`BatchInput`] with the provided `batch_hash` in the database and set the
    /// finalized block number to the provided block number.
    ///
    /// Errors if the [`BatchInput`] associated with the provided `batch_hash` is not found in the
    /// database, this method logs and returns an error.
    pub async fn finalize_batch_input(
        &self,
        batch_hash: B256,
        block_number: u64,
    ) -> Result<(), DatabaseError> {
        if let Some(batch) = models::batch_input::Entity::find()
            .filter(models::batch_input::Column::Hash.eq(batch_hash.to_vec()))
            .one(&self.connection)
            .await?
        {
            tracing::trace!(target: "scroll::db", batch_hash = ?batch_hash, block_number, "Finalizing batch input in database.");
            let mut batch: models::batch_input::ActiveModel = batch.into();
            batch.finalized_block_number = Set(Some(block_number as i64));
            batch.update(&self.connection).await?;
        } else {
            tracing::error!(
                target: "scroll::db",
                batch_hash = ?batch_hash,
                block_number,
                "Batch not found in DB when trying to finalize."
            );
            return Err(DatabaseError::BatchNotFound(batch_hash));
        }

        Ok(())
    }

    /// Get a [`BatchInput`] from the database by its batch index.
    pub async fn get_batch_input_by_batch_index(
        &self,
        batch_index: u64,
    ) -> Result<Option<BatchInput>, DatabaseError> {
        Ok(models::batch_input::Entity::find_by_id(
            TryInto::<i64>::try_into(batch_index).expect("index should fit in i64"),
        )
        .one(&self.connection)
        .await
        .map(|x| x.map(Into::into))?)
    }

    /// Delete all [`BatchInput`]s with a block number greater than the provided block number.
    pub async fn delete_batch_inputs_gt(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting batch inputs greater than block number.");
        Ok(models::batch_input::Entity::delete_many()
            .filter(models::batch_input::Column::BlockNumber.gt(block_number as i64))
            .exec(&self.connection)
            .await
            .map(|_| ())?)
    }

    /// Get an iterator over all [`BatchInput`]s in the database.
    pub async fn get_batch_inputs<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<BatchInput, DbErr>> + 'a, DbErr> {
        Ok(models::batch_input::Entity::find()
            .stream(&self.connection)
            .await?
            .map(|res| res.map(Into::into)))
    }

    /// Insert an [`L1MessageWithBlockNumber`] into the database.
    pub async fn insert_l1_message(
        &self,
        l1_message: L1MessageWithBlockNumber,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", queue_index = l1_message.transaction.queue_index, "Inserting L1 message into database.");
        let l1_message: models::l1_message::ActiveModel = l1_message.into();
        l1_message.insert(&self.connection).await?;
        Ok(())
    }

    /// Delete all [`L1MessageWithBlockNumber`]s with a block number greater than the provided block
    /// number.
    pub async fn delete_l1_messages_gt(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting L1 messages greater than block number.");
        Ok(models::l1_message::Entity::delete_many()
            .filter(models::l1_message::Column::BlockNumber.gt(block_number as i64))
            .exec(&self.connection)
            .await
            .map(|_| ())?)
    }

    /// Get a [`L1MessageWithBlockNumber`] from the database by its message queue index.
    pub async fn get_l1_message(
        &self,
        queue_index: u64,
    ) -> Result<Option<L1MessageWithBlockNumber>, DatabaseError> {
        Ok(models::l1_message::Entity::find_by_id(queue_index as i64)
            .one(&self.connection)
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Gets an iterator over all [`L1MessageWithBlockNumber`]s in the database.
    pub async fn get_l1_messages<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<L1MessageWithBlockNumber, DbErr>> + 'a, DatabaseError>
    {
        Ok(models::l1_message::Entity::find()
            .stream(&self.connection)
            .await?
            .map(|res| res.map(Into::into)))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_utils::setup_test_db;
    use arbitrary::{Arbitrary, Unstructured};
    use futures::StreamExt;
    use rand::Rng;
    use rollup_node_primitives::{BatchInputV1, BatchInputV2};

    #[tokio::test]
    async fn test_database_round_trip_batch_input() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a random BatchInputV1.
        let batch_input_v1 = BatchInputV1::arbitrary(&mut u).unwrap();
        let batch_input = BatchInput::BatchInputDataV1(batch_input_v1);

        // Round trip the BatchInput through the database.
        db.insert_batch_input(batch_input.clone()).await.unwrap();
        let batch_input_from_db =
            db.get_batch_input_by_batch_index(batch_input.batch_index()).await.unwrap().unwrap();
        assert_eq!(batch_input, batch_input_from_db);

        // Generate a random BatchInputV2.
        let batch_input_v2 = BatchInputV2::arbitrary(&mut u).unwrap();
        let batch_input = BatchInput::BatchInputDataV2(batch_input_v2);

        // Round trip the BatchInput through the database.
        db.insert_batch_input(batch_input.clone()).await.unwrap();
        let batch_input_from_db =
            db.get_batch_input_by_batch_index(batch_input.batch_index()).await.unwrap().unwrap();
        assert_eq!(batch_input, batch_input_from_db);
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
