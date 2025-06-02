use super::{models, DatabaseError};
use crate::DatabaseConnectionProvider;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::B256;
use futures::{Stream, StreamExt};
use rollup_node_primitives::{
    BatchCommitData, BatchInfo, BlockInfo, L1MessageEnvelope, L2BlockInfoWithL1Messages,
};
use scroll_alloy_rpc_types_engine::BlockDataHint;
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Set,
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
    async fn delete_batches_gt(&self, block_number: u64) -> Result<u64, DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting batch inputs greater than block number.");
        Ok(models::batch_commit::Entity::delete_many()
            .filter(models::batch_commit::Column::BlockNumber.gt(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|x| x.rows_affected)?)
    }

    /// Get an iterator over all [`BatchCommitData`]s in the database.
    async fn get_batches<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<BatchCommitData, DatabaseError>> + 'a, DatabaseError>
    {
        Ok(models::batch_commit::Entity::find()
            .stream(self.get_connection())
            .await?
            .map(|res| Ok(res.map(Into::into)?)))
    }

    /// Get the latest batch commit from the database.
    async fn get_latest_batch_commit(&self) -> Result<Option<BatchCommitData>, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Fetching latest batch commit from database.");
        Ok(models::batch_commit::Entity::find()
            .order_by_desc(models::batch_commit::Column::Index)
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Insert an [`L1MessageEnvelope`] into the database.
    async fn insert_l1_message(&self, l1_message: L1MessageEnvelope) -> Result<(), DatabaseError> {
        let l1_message: models::l1_message::ActiveModel = l1_message.into();
        l1_message.insert(self.get_connection()).await?;
        Ok(())
    }

    /// Delete all [`L1MessageEnvelope`]s with a block number greater than the provided block
    /// number and return them.
    async fn delete_l1_messages_gt(
        &self,
        l1_block_number: u64,
    ) -> Result<Vec<L1MessageEnvelope>, DatabaseError> {
        tracing::trace!(
            target: "scroll::db",
            l1_block_number,
            "Fetching L1 messages to delete with block number greater than provided."
        );

        let removed_messages = models::l1_message::Entity::find()
            .filter(models::l1_message::Column::L1BlockNumber.gt(l1_block_number as i64))
            .all(self.get_connection())
            .await?;
        models::l1_message::Entity::delete_many()
            .filter(models::l1_message::Column::L1BlockNumber.gt(l1_block_number as i64))
            .exec(self.get_connection())
            .await?;

        Ok(removed_messages.into_iter().map(Into::into).collect())
    }

    /// Get a [`L1MessageEnvelope`] from the database by its message queue index.
    async fn get_l1_message_by_index(
        &self,
        queue_index: u64,
    ) -> Result<Option<L1MessageEnvelope>, DatabaseError> {
        Ok(models::l1_message::Entity::find_by_id(queue_index as i64)
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Get a [`L1MessageEnvelope`] from the database by its message queue hash.
    async fn get_l1_message_by_hash(
        &self,
        queue_hash: B256,
    ) -> Result<Option<L1MessageEnvelope>, DatabaseError> {
        Ok(models::l1_message::Entity::find()
            .filter(
                Condition::all()
                    .add(models::l1_message::Column::QueueHash.is_not_null())
                    .add(models::l1_message::Column::QueueHash.eq(queue_hash.to_vec())),
            )
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Gets an iterator over all [`L1MessageEnvelope`]s in the database.
    async fn get_l1_messages<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<L1MessageEnvelope, DatabaseError>> + 'a, DatabaseError>
    {
        Ok(models::l1_message::Entity::find()
            .stream(self.get_connection())
            .await?
            .map(|res| Ok(res.map(Into::into)?)))
    }

    /// Get the latest [`L1MessageEnvelope`] from the database.
    async fn get_latest_l1_message(&self) -> Result<Option<L1MessageEnvelope>, DatabaseError> {
        Ok(models::l1_message::Entity::find()
            .order_by_desc(models::l1_message::Column::L1BlockNumber)
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Get the extra data for the provided [`BlockId`].
    async fn get_l2_block_data_hint(
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

    /// Get a [`BlockInfo`] from the database by its block number.
    async fn get_l2_block_info_by_number(
        &self,
        block_number: u64,
    ) -> Result<Option<BlockInfo>, DatabaseError> {
        Ok(models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BlockNumber.eq(block_number as i64))
            .one(self.get_connection())
            .await
            .map(|x| {
                x.map(|x| {
                    let (block_info, _maybe_batch_info): (BlockInfo, Option<BatchInfo>) = x.into();
                    block_info
                })
            })?)
    }

    /// Get the latest safe L2 [`BlockInfo`] from the database.
    async fn get_latest_safe_l2_block(&self) -> Result<Option<BlockInfo>, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Fetching latest safe L2 block from database.");
        Ok(models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BatchIndex.is_not_null())
            .order_by_desc(models::l2_block::Column::BlockNumber)
            .one(self.get_connection())
            .await
            .map(|x| x.map(|x| x.block_info()))?)
    }

    /// Get the latest L2 [`BlockInfo`] from the database.
    async fn get_latest_l2_block(&self) -> Result<Option<BlockInfo>, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Fetching latest L2 block from database.");
        Ok(models::l2_block::Entity::find()
            .order_by_desc(models::l2_block::Column::BlockNumber)
            .one(self.get_connection())
            .await
            .map(|x| x.map(|x| x.block_info()))?)
    }

    /// Delete all L2 blocks with a block number greater than the provided block number.
    async fn delete_l2_blocks_gt(&self, block_number: u64) -> Result<u64, DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting L2 blocks greater than provided block number.");
        Ok(models::l2_block::Entity::delete_many()
            .filter(models::l2_block::Column::BlockNumber.gt(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|x| x.rows_affected)?)
    }

    /// Insert a new block in the database.
    async fn insert_block(
        &self,
        block_info: L2BlockInfoWithL1Messages,
        batch_info: Option<BatchInfo>,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(
            target: "scroll::db",
            batch_hash = ?batch_info.as_ref().map(|b| b.hash),
            batch_index = batch_info.as_ref().map(|b| b.index),
            block_number = block_info.block_info.number,
            block_hash = ?block_info.block_info.hash,
            "Inserting block into database."
        );
        let l2_block: models::l2_block::ActiveModel = (block_info.block_info, batch_info).into();
        models::l2_block::Entity::insert(l2_block)
            .on_conflict(
                OnConflict::column(models::l2_block::Column::BlockNumber)
                    .update_columns([
                        models::l2_block::Column::BatchHash,
                        models::l2_block::Column::BatchIndex,
                    ])
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await?;

        tracing::trace!(
            target: "scroll::db",
            block_number = block_info.block_info.number,
            l1_messages = ?block_info.l1_messages,
            "Updating executed L1 messages from block with L2 block number in the database."
        );
        models::l1_message::Entity::update_many()
            .col_expr(
                models::l1_message::Column::L2BlockNumber,
                Expr::value(block_info.block_info.number as i64),
            )
            .filter(
                models::l1_message::Column::Hash
                    .is_in(block_info.l1_messages.iter().map(|x| x.to_vec())),
            )
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    /// Returns the highest L2 block originating from the provided `batch_hash` or the highest block
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
            Ok(models::l2_block::Entity::find()
                .filter(models::l2_block::Column::BatchIndex.lte(index))
                .order_by_desc(models::l2_block::Column::BlockNumber)
                .one(self.get_connection())
                .await?
                .map(|model| model.block_info()))
        } else {
            Ok(None)
        }
    }
}

impl<T> DatabaseOperations for T where T: DatabaseConnectionProvider {}
