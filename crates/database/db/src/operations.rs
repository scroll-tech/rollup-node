use super::{models, DatabaseError};
use crate::DatabaseConnectionProvider;

use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::B256;
use futures::{Stream, StreamExt};
use rollup_node_primitives::{BatchCommitData, BatchInfo, BlockInfo, L1MessageEnvelope};
use scroll_alloy_rpc_types_engine::BlockDataHint;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};

/// The [`DatabaseOperations`] trait provides methods for interacting with the database.
#[async_trait::async_trait]
pub trait DatabaseOperations: DatabaseConnectionProvider {
    /// Insert a [`BatchCommitData`] into the database.
    async fn insert_batch(&self, batch_commit: BatchCommitData) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", batch_hash = ?batch_commit.hash, batch_index = batch_commit.index, "Inserting batch input into database.");
        let batch_commit: models::batch_commit::ActiveModel = batch_commit.into();
        batch_commit.insert(self.get_connection()).await?;
        Ok(())
    }

    /// Finalize a [`BatchCommitData`] with the provided `batch_hash` in the database and set the
    /// finalized block number to the provided block number.
    ///
    /// Errors if the [`BatchCommitData`] associated with the provided `batch_hash` is not found in
    /// the database, this method logs and returns an error.
    async fn finalize_batch(
        &self,
        batch_hash: B256,
        block_number: u64,
    ) -> Result<(), DatabaseError> {
        if let Some(batch) = models::batch_commit::Entity::find()
            .filter(models::batch_commit::Column::Hash.eq(batch_hash.to_vec()))
            .one(self.get_connection())
            .await?
        {
            tracing::trace!(target: "scroll::db", batch_hash = ?batch_hash, block_number, "Finalizing batch commit in database.");
            let mut batch: models::batch_commit::ActiveModel = batch.into();
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

    /// Get a [`BatchCommitData`] from the database by its batch index.
    async fn get_batch_by_index(
        &self,
        batch_index: u64,
    ) -> Result<Option<BatchCommitData>, DatabaseError> {
        Ok(models::batch_commit::Entity::find_by_id(
            TryInto::<i64>::try_into(batch_index).expect("index should fit in i64"),
        )
        .one(self.get_connection())
        .await
        .map(|x| x.map(Into::into))?)
    }

    /// Get the newest finalized batch hash up to or at the provided height.
    async fn get_finalized_batch_hash_at_height(
        &self,
        height: u64,
    ) -> Result<Option<B256>, DatabaseError> {
        Ok(models::batch_commit::Entity::find()
            .filter(
                Condition::all()
                    .add(models::batch_commit::Column::FinalizedBlockNumber.is_not_null())
                    .add(models::batch_commit::Column::FinalizedBlockNumber.lte(height)),
            )
            .order_by_desc(models::batch_commit::Column::Index)
            .select_only()
            .column(models::batch_commit::Column::Hash)
            .into_tuple::<Vec<u8>>()
            .one(self.get_connection())
            .await
            .map(|x| x.map(|x| B256::from_slice(&x)))?)
    }

    /// Delete all [`BatchCommitData`]s with a block number greater than the provided block number.
    async fn delete_batches_gt(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting batch inputs greater than block number.");
        Ok(models::batch_commit::Entity::delete_many()
            .filter(models::batch_commit::Column::BlockNumber.gt(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    /// Get an iterator over all [`BatchCommitData`]s in the database.
    async fn get_batches<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<BatchCommitData, DbErr>> + 'a, DbErr> {
        Ok(models::batch_commit::Entity::find()
            .stream(self.get_connection())
            .await?
            .map(|res| res.map(Into::into)))
    }

    /// Insert an [`L1MessageWithBlockNumber`] into the database.
    async fn insert_l1_message(&self, l1_message: L1MessageEnvelope) -> Result<(), DatabaseError> {
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
    ) -> Result<Option<L1MessageEnvelope>, DatabaseError> {
        Ok(models::l1_message::Entity::find_by_id(queue_index as i64)
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Gets an iterator over all [`L1MessageWithBlockNumber`]s in the database.
    async fn get_l1_messages<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<L1MessageEnvelope, DbErr>> + 'a, DatabaseError> {
        Ok(models::l1_message::Entity::find()
            .stream(self.get_connection())
            .await?
            .map(|res| res.map(Into::into)))
    }

    /// Get the extra data for the provided [`BlockId`].
    async fn get_block_data(
        &self,
        block_id: BlockId,
    ) -> Result<Option<BlockDataHint>, DatabaseError> {
        let filter = match block_id {
            BlockId::Hash(hash) => models::block_data::Column::Hash.eq(hash.block_hash.to_vec()),
            BlockId::Number(BlockNumberOrTag::Number(number)) => {
                models::block_data::Column::Number.eq(number as i64)
            }
            x => return Err(DatabaseError::BlockNotFound(x)),
        };
        Ok(models::block_data::Entity::find()
            .filter(filter)
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Insert a new derived block line in the database.
    async fn insert_derived_block(
        &self,
        batch_info: BatchInfo,
        block_info: BlockInfo,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(
            target: "scroll::db",
            batch_hash = ?batch_info.hash,
            batch_index = batch_info.index,
            block_number = block_info.number,
            block_hash = ?block_info.hash,
            "Inserting derived block into database."
        );
        let derived_block: models::derived_block::ActiveModel = (batch_info, block_info).into();
        derived_block.insert(self.get_connection()).await?;

        Ok(())
    }

    /// Returns the highest L2 block originating from the provided batch_hash or the highest block
    /// for the batch's index.
    async fn get_highest_block_for_batch(
        &self,
        batch_hash: B256,
    ) -> Result<Option<BlockInfo>, DatabaseError> {
        let index = models::batch_commit::Entity::find()
            .filter(models::batch_commit::Column::Hash.eq(batch_hash.to_vec()))
            .select_only()
            .column(models::batch_commit::Column::Index)
            .into_tuple::<i32>()
            .one(self.get_connection())
            .await?;

        if let Some(index) = index {
            Ok(models::derived_block::Entity::find()
                .filter(models::derived_block::Column::BatchIndex.lte(index))
                .order_by_desc(models::derived_block::Column::BlockNumber)
                .one(self.get_connection())
                .await?
                .map(|model| model.block_info()))
        } else {
            Ok(None)
        }
    }
}

impl<T> DatabaseOperations for T where T: DatabaseConnectionProvider {}
