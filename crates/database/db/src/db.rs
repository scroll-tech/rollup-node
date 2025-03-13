use super::models;
use alloy_primitives::B256;
use futures::{Stream, StreamExt};
use rollup_node_primitives::{BatchInput, L1MessageWithBlockNumber};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, Database as SeaOrmDatabase, DatabaseConnection,
    DatabaseTransaction, DbErr, EntityTrait, QueryFilter, Set, TransactionTrait,
};

/// The [`Database`] struct is responsible for interacting with the database.
#[derive(Debug)]
pub struct Database {
    pub(crate) connection: DatabaseConnection,
}

impl Database {
    /// Creates a new [`Database`] instance associated with the provided database URL.
    pub async fn new(database_url: &str) -> Result<Self, DbErr> {
        let connection = SeaOrmDatabase::connect(database_url).await?;
        Ok(Self { connection })
    }

    /// Returns a reference to the underlying database connection.
    pub fn connection(&self) -> &DatabaseConnection {
        &self.connection
    }

    /// Creates a new transaction against the database.
    pub async fn tx(&self) -> Result<DatabaseTransaction, DbErr> {
        self.connection.begin().await
    }

    /// Insert a [`BatchInput`] into the database.
    pub async fn insert_batch_input<C: ConnectionTrait>(
        &self,
        conn: &C,
        batch_input: BatchInput,
    ) -> Result<models::batch_input::Model, DbErr> {
        let batch_input: models::batch_input::ActiveModel = batch_input.into();
        batch_input.insert(conn).await
    }

    /// Finalize a [`BatchInput`] in the database and set the finalized block number to the provided
    /// block number.
    pub async fn finalize_batch_input<C: ConnectionTrait>(
        &self,
        conn: &C,
        batch_hash: B256,
        block_number: u64,
    ) -> Result<(), DbErr> {
        if let Some(batch) = models::batch_input::Entity::find()
            .filter(models::batch_input::Column::Hash.eq(batch_hash.to_vec())) // Filter by batch_hash
            .one(conn)
            .await?
        // Retrieve single row
        {
            let mut batch: models::batch_input::ActiveModel = batch.into();
            batch.finalized_block_number = Set(Some(block_number as i64));
            batch.update(conn).await?; // Update only this field
        } else {
            tracing::warn!(
                target: "scroll::db",
                batch_hash = ?batch_hash,
                block_number,
                "Batch not found in DB when trying to finalize."
            );
        }

        Ok(())
    }

    /// Get a [`BatchInput`] from the database by its batch index.
    pub async fn get_batch_input_by_batch_index<C: ConnectionTrait>(
        &self,
        conn: &C,
        batch_index: u64,
    ) -> Result<Option<BatchInput>, DbErr> {
        models::batch_input::Entity::find_by_id(
            TryInto::<i64>::try_into(batch_index).expect("index should fit in i64"),
        )
        .one(conn)
        .await
        .map(|x| x.map(Into::into))
    }

    /// Delete all [`BatchInput`]s with a block number greater than the provided block number.
    pub async fn delete_batch_inputs_gt<C: ConnectionTrait>(
        &self,
        conn: &C,
        block_number: u64,
    ) -> Result<(), DbErr> {
        models::batch_input::Entity::delete_many()
            .filter(models::batch_input::Column::BlockNumber.gt(block_number as i64))
            .exec(conn)
            .await
            .map(|_| ())
    }

    /// Get an iterator over all [`BatchInput`]s in the database.
    pub async fn get_batch_inputs(
        &self,
    ) -> Result<impl Stream<Item = Result<BatchInput, DbErr>> + use<'_>, DbErr> {
        Ok(models::batch_input::Entity::find()
            .stream(&self.connection)
            .await?
            .map(|res| res.map(Into::into)))
    }

    /// Insert an [`L1MessageWithBlockNumber`] into the database.
    pub async fn insert_l1_message<C: ConnectionTrait>(
        &self,
        conn: &C,
        l1_message: L1MessageWithBlockNumber,
    ) -> Result<(), DbErr> {
        let l1_message: models::l1_message::ActiveModel = l1_message.into();
        l1_message.insert(conn).await?;
        Ok(())
    }

    /// Delete all [`L1MessageWithBlockNumber`]s with a block number greater than the provided block
    /// number.
    pub async fn delete_l1_messages_gt<C: ConnectionTrait>(
        &self,
        conn: &C,
        block_number: u64,
    ) -> Result<(), DbErr> {
        models::l1_message::Entity::delete_many()
            .filter(models::l1_message::Column::BlockNumber.gt(block_number as i64))
            .exec(conn)
            .await
            .map(|_| ())
    }

    /// Get a [`L1Message`] from the database by its message queue index.
    pub async fn get_l1_message(
        &self,
        queue_index: u64,
    ) -> Result<Option<L1MessageWithBlockNumber>, DbErr> {
        models::l1_message::Entity::find_by_id(queue_index as i64)
            .one(&self.connection)
            .await
            .map(|x| x.map(Into::into))
    }

    /// Gets an iterator over all [`L1Message`]s in the database.
    pub async fn get_l1_messages(
        &self,
    ) -> Result<impl Stream<Item = Result<L1MessageWithBlockNumber, DbErr>> + use<'_>, DbErr> {
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
        db.insert_batch_input(db.connection(), batch_input.clone()).await.unwrap();
        let batch_input_from_db = db
            .get_batch_input_by_batch_index(db.connection(), batch_input.batch_index())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(batch_input, batch_input_from_db);

        // Generate a random BatchInputV2.
        let batch_input_v2 = BatchInputV2::arbitrary(&mut u).unwrap();
        let batch_input = BatchInput::BatchInputDataV2(batch_input_v2);

        // Round trip the BatchInput through the database.
        db.insert_batch_input(db.connection(), batch_input.clone()).await.unwrap();
        let batch_input_from_db = db
            .get_batch_input_by_batch_index(db.connection(), batch_input.batch_index())
            .await
            .unwrap()
            .unwrap();
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
        db.insert_l1_message(db.connection(), l1_message.clone()).await.unwrap();
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
        db.insert_l1_message(&tx, l1_message_1.clone()).await.unwrap();
        db.insert_l1_message(&tx, l1_message_2.clone()).await.unwrap();
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
        db.insert_l1_message(db.connection(), l1_message_1.clone()).await.unwrap();
        db.insert_l1_message(db.connection(), l1_message_2.clone()).await.unwrap();

        // collect the L1Messages
        let l1_messages =
            db.get_l1_messages().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;

        // Apply the assertions.
        assert!(l1_messages.contains(&l1_message_1));
        assert!(l1_messages.contains(&l1_message_2));
    }
}
