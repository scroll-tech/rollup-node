use crate::{
    database::SingleOrMultiple,
    request::{DatabaseCall, DatabaseRequest},
};
use sea_orm::{prelude::async_trait, sea_query::OnConflict, DbErr, EntityTrait};
use tower::Service;

pub mod database;
pub mod models;
pub mod request;
pub mod retry;
pub mod tx;

#[auto_impl::auto_impl(Arc)]
pub trait DatabaseConnectionProvider {
    /// The type of the database connection.
    type Connection: sea_orm::ConnectionTrait + sea_orm::StreamTrait;

    /// Returns a reference to the database connection that implements the `ConnectionTrait` and
    /// `StreamTrait` traits.
    fn get_connection(&self) -> &Self::Connection;
}

type BatchCommitMapper = Box<dyn FnOnce(SingleOrMultiple<models::Model>) -> BatchCommitData>;

pub trait DatabaseReadOperations {
    fn get_batch_by_index(
        &self,
        batch_index: u64,
    ) -> DatabaseCall<models::Entity, BatchCommitData, BatchCommitMapper, Self>
    where
        Self: Sized,
        Self: Service<DatabaseRequest<models::Entity>>;
}

#[async_trait::async_trait]
pub trait DatabaseWriteOperations {
    /// Insert a [`BatchCommitData`] into the database.
    async fn insert_batch(&self, batch_commit: BatchCommitData) -> Result<(), DatabaseError> {
        let batch_commit: models::ActiveModel = batch_commit.into();
        Ok(models::Entity::insert(batch_commit)
            .on_conflict(
                OnConflict::column(models::Column::Index)
                    .update_columns(vec![
                        models::Column::BlockNumber,
                        models::Column::BlockTimestamp,
                        models::Column::FinalizedBlockNumber,
                    ])
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }
}

/// The error type for database operations.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    /// A database error occurred.
    #[error("database error: {0}")]
    DatabaseError(#[from] DbErr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchCommitData {
    /// The index of the batch.
    pub index: u64,
    /// The block number in which the batch was committed.
    pub block_number: u64,
    /// The block timestamp in which the batch was committed.
    pub block_timestamp: u64,
    /// The block number at which the batch finalized event was emitted.
    pub finalized_block_number: Option<u64>,
}
