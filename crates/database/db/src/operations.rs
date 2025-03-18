use super::{models, DatabaseError};
use crate::DatabaseConnectionProvider;
use alloy_primitives::B256;
use futures::{Stream, StreamExt};
use rollup_node_primitives::{BatchInput, L1MessageWithBlockNumber};
use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set};

/// The [`DatabaseOperations`] trait provides methods for interacting with the database.
#[async_trait::async_trait]
pub trait DatabaseOperations: DatabaseConnectionProvider {
    /// Insert a [`BatchInput`] into the database.
    async fn insert_batch_input(&self, batch_input: BatchInput) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", batch_hash = ?batch_input.batch_hash(), batch_index = batch_input.batch_index(), "Inserting batch input into database.");
        let batch_input: models::batch_input::ActiveModel = batch_input.into();
        batch_input.insert(self.get_connection()).await?;
        Ok(())
    }

    /// Finalize a [`BatchInput`] with the provided `batch_hash` in the database and set the
    /// finalized block number to the provided block number.
    ///
    /// Errors if the [`BatchInput`] associated with the provided `batch_hash` is not found in the
    /// database, this method logs and returns an error.
    async fn finalize_batch_input(
        &self,
        batch_hash: B256,
        block_number: u64,
    ) -> Result<(), DatabaseError> {
        if let Some(batch) = models::batch_input::Entity::find()
            .filter(models::batch_input::Column::Hash.eq(batch_hash.to_vec()))
            .one(self.get_connection())
            .await?
        {
            tracing::trace!(target: "scroll::db", batch_hash = ?batch_hash, block_number, "Finalizing batch input in database.");
            let mut batch: models::batch_input::ActiveModel = batch.into();
            batch.finalized_block_number = Set(Some(block_number as i64));
            batch.update(self.get_connection()).await?;
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
    async fn get_batch_input_by_batch_index(
        &self,
        batch_index: u64,
    ) -> Result<Option<BatchInput>, DatabaseError> {
        Ok(models::batch_input::Entity::find_by_id(
            TryInto::<i64>::try_into(batch_index).expect("index should fit in i64"),
        )
        .one(self.get_connection())
        .await
        .map(|x| x.map(Into::into))?)
    }

    /// Delete all [`BatchInput`]s with a block number greater than the provided block number.
    async fn delete_batch_inputs_gt(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting batch inputs greater than block number.");
        Ok(models::batch_input::Entity::delete_many()
            .filter(models::batch_input::Column::BlockNumber.gt(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    /// Get an iterator over all [`BatchInput`]s in the database.
    async fn get_batch_inputs<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<BatchInput, DbErr>> + 'a, DbErr> {
        Ok(models::batch_input::Entity::find()
            .stream(self.get_connection())
            .await?
            .map(|res| res.map(Into::into)))
    }

    /// Insert an [`L1MessageWithBlockNumber`] into the database.
    async fn insert_l1_message(
        &self,
        l1_message: L1MessageWithBlockNumber,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", queue_index = l1_message.transaction.queue_index, "Inserting L1 message into database.");
        let l1_message: models::l1_message::ActiveModel = l1_message.into();
        l1_message.insert(self.get_connection()).await?;
        Ok(())
    }

    /// Delete all [`L1MessageWithBlockNumber`]s with a block number greater than the provided block
    /// number.
    async fn delete_l1_messages_gt(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting L1 messages greater than block number.");
        Ok(models::l1_message::Entity::delete_many()
            .filter(models::l1_message::Column::BlockNumber.gt(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    /// Get a [`L1MessageWithBlockNumber`] from the database by its message queue index.
    async fn get_l1_message(
        &self,
        queue_index: u64,
    ) -> Result<Option<L1MessageWithBlockNumber>, DatabaseError> {
        Ok(models::l1_message::Entity::find_by_id(queue_index as i64)
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Gets an iterator over all [`L1MessageWithBlockNumber`]s in the database.
    async fn get_l1_messages<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<L1MessageWithBlockNumber, DbErr>> + 'a, DatabaseError>
    {
        Ok(models::l1_message::Entity::find()
            .stream(self.get_connection())
            .await?
            .map(|res| res.map(Into::into)))
    }
}

impl<T> DatabaseOperations for T where T: DatabaseConnectionProvider {}
