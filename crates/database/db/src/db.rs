use super::models;
use scroll_primitives::{BatchInput, L1Message};
use sea_orm::{
    ActiveModelTrait, Database as SeaOrmDatabase, DatabaseConnection, DbErr, EntityTrait,
};

/// The [`Database`] struct is responsible for interacting with the database.
#[derive(Debug)]
pub struct Database {
    connection: DatabaseConnection,
}

impl Database {
    /// Creates a new [`Database`] instance associated with the provided database URL.
    pub async fn new(database_url: &str) -> Result<Self, DbErr> {
        let connection = SeaOrmDatabase::connect(database_url).await?;
        Ok(Self { connection })
    }

    /// Insert a [`BatchInput`] into the database.
    pub async fn insert_batch_input(
        &self,
        batch_input: BatchInput,
    ) -> Result<models::batch_input::Model, DbErr> {
        let batch_input: models::batch_input::ActiveModel = batch_input.into();
        batch_input.insert(&self.connection).await
    }

    /// Get a [`BatchInput`] from the database by its batch index.
    pub async fn get_batch_input_by_batch_index(
        &self,
        batch_index: u64,
    ) -> Result<Option<BatchInput>, DbErr> {
        models::batch_input::Entity::find_by_id(
            TryInto::<i64>::try_into(batch_index).expect("index should fit in i64"),
        )
        .one(&self.connection)
        .await
        .map(|x| x.map(Into::into))
    }

    /// Insert an [`L1Message`] into the database.
    pub async fn insert_l1_message(&self, l1_message: L1Message) -> Result<(), DbErr> {
        let l1_message: models::l1_message::ActiveModel = l1_message.into();
        l1_message.insert(&self.connection).await?;
        Ok(())
    }

    /// Get a [`L1Message`] from the database by its message queue index.
    pub async fn get_l1_message(&self, queue_index: u64) -> Result<Option<L1Message>, DbErr> {
        models::l1_message::Entity::find_by_id(queue_index as i64)
            .one(&self.connection)
            .await
            .map(|x| x.map(Into::into))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use arbitrary::{Arbitrary, Unstructured};
    use migration::{Migrator, MigratorTrait};
    use rand::Rng;
    use scroll_primitives::{BatchInputV1, BatchInputV2};

    async fn setup_test_db() -> Database {
        let database_url = "sqlite::memory:";
        let connection = sea_orm::Database::connect(database_url).await.unwrap();
        Migrator::up(&connection, None).await.unwrap();
        let db = Database { connection };
        db
    }

    #[tokio::test]
    async fn test_database_round_trip_batch_input() {
        // Set up the test database.
        let db = setup_test_db().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::thread_rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a random BatchInputV1.
        let batch_input_v1 = BatchInputV1::arbitrary(&mut u).unwrap();
        let batch_input = BatchInput::BatchInputV1(batch_input_v1);

        // Round trip the BatchInput through the database.
        db.insert_batch_input(batch_input.clone()).await.unwrap();
        let batch_input_from_db =
            db.get_batch_input_by_batch_index(batch_input.batch_index()).await.unwrap().unwrap();
        assert_eq!(batch_input, batch_input_from_db);

        // Generate a random BatchInputV2.
        let batch_input_v2 = BatchInputV2::arbitrary(&mut u).unwrap();
        let batch_input = BatchInput::BatchInputV2(batch_input_v2);

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
        rand::thread_rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a random L1Message.
        let l1_message = L1Message::arbitrary(&mut u).unwrap();

        // Round trip the L1Message through the database.
        db.insert_l1_message(l1_message.clone()).await.unwrap();
        let l1_message_from_db =
            db.get_l1_message(l1_message.transaction.queue_index).await.unwrap().unwrap();
        assert_eq!(l1_message, l1_message_from_db);
    }
}
