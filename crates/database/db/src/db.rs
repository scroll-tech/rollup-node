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
    use crate::{operations::DatabaseOperations, test_utils::setup_test_db};
    use arbitrary::{Arbitrary, Unstructured};
    use futures::StreamExt;
    use rand::Rng;
    use rollup_node_primitives::{
        BatchInput, BatchInputV1, BatchInputV2, L1MessageWithBlockNumber,
    };

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
