use super::{models, DatabaseError};
use crate::{ReadConnectionProvider, WriteConnectionProvider};

use alloy_primitives::{Signature, B256};
use rollup_node_primitives::{
    BatchCommitData, BatchConsolidationOutcome, BatchInfo, BatchStatus, BlockInfo,
    L1BlockStartupInfo, L1MessageEnvelope, L2BlockInfoWithL1Messages, Metadata,
};
use scroll_alloy_rpc_types_engine::BlockDataHint;
use sea_orm::{
    sea_query::{CaseStatement, Expr, OnConflict},
    ColumnTrait, Condition, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use std::fmt;

/// The [`DatabaseWriteOperations`] trait provides write methods for interacting with the
/// database.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait DatabaseWriteOperations {
    /// Insert a [`BlockInfo`] representing an L1 block into the database.
    async fn insert_l1_block_info(&self, block_info: BlockInfo) -> Result<(), DatabaseError>;

    /// Remove all [`BlockInfo`]s representing L1 blocks with block numbers less than or equal to
    /// the provided block number.
    async fn remove_l1_block_info_leq(&self, block_info: u64) -> Result<(), DatabaseError>;

    /// Remove all [`BlockInfo`]s representing L1 blocks with block numbers greater than the
    /// provided block number.
    async fn remove_l1_block_info_gt(&self, block_number: u64) -> Result<(), DatabaseError>;

    /// Insert a [`BatchCommitData`] into the database.
    async fn insert_batch(&self, batch_commit: BatchCommitData) -> Result<(), DatabaseError>;

    /// Finalizes all [`BatchCommitData`] up to the provided `batch_index` by setting their
    /// finalized block number to the provided block number.
    async fn finalize_batches_up_to_index(
        &self,
        batch_index: u64,
        block_number: u64,
    ) -> Result<(), DatabaseError>;

    /// Set the latest L1 block number.
    async fn set_latest_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError>;

    /// Set the finalized L1 block number.
    async fn set_finalized_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError>;

    /// Set the processed L1 block number.
    async fn set_processed_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError>;

    /// Set the L2 head block number.
    async fn set_l2_head_block_number(&self, number: u64) -> Result<(), DatabaseError>;

    /// Fetches unprocessed batches up to the provided finalized L1 block number and updates their
    /// status to processing.
    async fn fetch_and_update_unprocessed_finalized_batches(
        &self,
        finalized_l1_block_number: u64,
    ) -> Result<Vec<BatchInfo>, DatabaseError>;

    /// Fetches unprocessed committed batches and updates their status to processing.
    async fn fetch_and_update_unprocessed_committed_batches(
        &self,
    ) -> Result<Vec<BatchInfo>, DatabaseError>;

    /// Delete all [`BatchCommitData`]s with a block number greater than the provided block number.
    async fn delete_batches_gt_block_number(&self, block_number: u64)
        -> Result<u64, DatabaseError>;

    /// Delete all effects of `BatchFinalization` events with a block number greater than the
    /// provided block number.
    async fn delete_batch_finalization_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<(), DatabaseError>;

    /// Sets the L1 block number of the batch revert associated with the provided batch index range.
    async fn set_batch_revert_block_number_for_batch_range(
        &self,
        start_index: u64,
        end_index: u64,
        block_info: BlockInfo,
    ) -> Result<(), DatabaseError>;

    /// Delete all batch reverts with a block number greater than the provided block number and
    /// returns the number of deleted reverts.
    async fn delete_batch_revert_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<u64, DatabaseError>;

    /// Finalize consolidated batches by updating their status in the database and returning the new
    /// finalized head.
    async fn finalize_consolidated_batches(
        &self,
        finalized_l1_block_number: u64,
    ) -> Result<Option<BlockInfo>, DatabaseError>;

    /// Set batches with processing status to committed status.
    async fn change_batch_processing_to_committed_status(&self) -> Result<(), DatabaseError>;

    /// Update the status of a batch identified by its hash.
    async fn update_batch_status(
        &self,
        batch_hash: B256,
        status: BatchStatus,
    ) -> Result<(), DatabaseError>;

    /// Delete all [`BatchCommitData`]s with a batch index greater than the provided index.
    async fn delete_batches_gt_batch_index(&self, batch_index: u64) -> Result<u64, DatabaseError>;

    /// Insert an [`L1MessageEnvelope`] into the database.
    async fn insert_l1_message(&self, l1_message: L1MessageEnvelope) -> Result<(), DatabaseError>;

    /// Sets the `skipped` column to true for the provided list of L1 messages queue indexes.
    async fn update_skipped_l1_messages(&self, indexes: Vec<u64>) -> Result<(), DatabaseError>;

    /// Delete all [`L1MessageEnvelope`]s with a block number greater than the provided block
    /// number and return them.
    async fn delete_l1_messages_gt(
        &self,
        l1_block_number: u64,
    ) -> Result<Vec<L1MessageEnvelope>, DatabaseError>;

    /// Returns the L1 block info required to start the L1 watcher on startup.
    async fn prepare_l1_watcher_start_info(&self) -> Result<L1BlockStartupInfo, DatabaseError>;

    /// Delete all L2 blocks with a block number greater than the provided block number.
    async fn delete_l2_blocks_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<u64, DatabaseError>;

    /// Delete all L2 blocks with a batch index greater than the batch index.
    async fn delete_l2_blocks_gt_batch_index(&self, batch_index: u64)
        -> Result<u64, DatabaseError>;

    /// Insert multiple blocks into the database.
    async fn insert_blocks(
        &self,
        blocks: Vec<BlockInfo>,
        batch_info: BatchInfo,
    ) -> Result<(), DatabaseError>;

    /// Insert the genesis block into the database.
    async fn insert_genesis_block(&self, genesis_hash: B256) -> Result<(), DatabaseError>;

    /// Update the executed L1 messages from the provided L2 blocks in the database.
    async fn update_l1_messages_from_l2_blocks(
        &self,
        blocks: Vec<L2BlockInfoWithL1Messages>,
    ) -> Result<(), DatabaseError>;

    /// Update the executed L1 messages with the provided L2 block number in the database.
    async fn update_l1_messages_with_l2_blocks(
        &self,
        block_info: Vec<L2BlockInfoWithL1Messages>,
    ) -> Result<(), DatabaseError>;

    /// Purge all L1 message to L2 block mappings from the database for blocks greater or equal to
    /// the provided block number. If the no block number is provided, purge mappings for all
    /// unsafe blocks.
    async fn purge_l1_message_to_l2_block_mappings(
        &self,
        block_number: Option<u64>,
    ) -> Result<(), DatabaseError>;

    /// Insert the outcome of a batch consolidation into the database.
    async fn insert_batch_consolidation_outcome(
        &self,
        outcome: BatchConsolidationOutcome,
    ) -> Result<(), DatabaseError>;

    /// Unwinds the chain orchestrator by deleting all indexed data greater than the provided L1
    /// block number.
    async fn unwind(&self, l1_block_number: u64) -> Result<UnwindResult, DatabaseError>;

    /// Store multiple block signatures in the database.
    async fn insert_signatures(
        &self,
        signatures: Vec<(B256, Signature)>,
    ) -> Result<(), DatabaseError>;

    /// Store a block signature in the database.
    /// TODO: remove this once we deprecated l2geth.
    async fn insert_signature(
        &self,
        block_hash: B256,
        signature: Signature,
    ) -> Result<(), DatabaseError>;
}

#[async_trait::async_trait]
impl<T: WriteConnectionProvider + ?Sized + Sync> DatabaseWriteOperations for T {
    async fn insert_l1_block_info(&self, block_info: BlockInfo) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", %block_info, "Inserting L1 block info into database.");
        let finalized_l1_block_number = self.get_finalized_l1_block_number().await?;
        if block_info.number <= finalized_l1_block_number {
            tracing::trace!(target: "scroll::db", %block_info, %finalized_l1_block_number, "L1 block info is less than or equal to finalized L1 block number, skipping insertion.");
            return Ok(());
        }

        let l1_block: models::l1_block::ActiveModel = block_info.into();
        Ok(models::l1_block::Entity::insert(l1_block)
            .on_conflict(
                OnConflict::column(models::l1_block::Column::BlockNumber)
                    .update_column(models::l1_block::Column::BlockHash)
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    async fn remove_l1_block_info_leq(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", %block_number, "Removing L1 block info less than or equal to provided block number from database.");
        Ok(models::l1_block::Entity::delete_many()
            .filter(models::l1_block::Column::BlockNumber.lte(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    async fn remove_l1_block_info_gt(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number = block_number, "Removing L1 block info greater than provided block number from database.");
        Ok(models::l1_block::Entity::delete_many()
            .filter(models::l1_block::Column::BlockNumber.gt(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    async fn insert_batch(&self, batch_commit: BatchCommitData) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", batch_hash = ?batch_commit.hash, batch_index = batch_commit.index, "Inserting batch input into database.");
        let batch_commit: models::batch_commit::ActiveModel = batch_commit.into();
        Ok(models::batch_commit::Entity::insert(batch_commit)
            .on_conflict(
                OnConflict::column(models::batch_commit::Column::Hash)
                    .update_columns(vec![
                        models::batch_commit::Column::Index,
                        models::batch_commit::Column::BlockNumber,
                        models::batch_commit::Column::BlockTimestamp,
                        models::batch_commit::Column::Calldata,
                        models::batch_commit::Column::BlobHash,
                        models::batch_commit::Column::FinalizedBlockNumber,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(self.get_connection())
            .await
            .map(|_| ())?)
    }

    async fn delete_batch_finalization_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(
            target: "scroll::db",
            block_number,
            "Deleting batch finalization effects greater than block number."
        );

        models::batch_commit::Entity::update_many()
            .filter(models::batch_commit::Column::FinalizedBlockNumber.gt(block_number as i64))
            .col_expr(models::batch_commit::Column::FinalizedBlockNumber, Expr::value(None::<i64>))
            .exec(self.get_connection())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    async fn set_batch_revert_block_number_for_batch_range(
        &self,
        start_index: u64,
        end_index: u64,
        block_info: BlockInfo,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", ?start_index, ?end_index, %block_info, "Setting batch revert block number for batch index range in database.");

        // Define the filter to select the appropriate batches.
        let filter = Condition::all()
            .add(models::batch_commit::Column::Index.gte(start_index as i64))
            .add(models::batch_commit::Column::Index.lte(end_index as i64))
            .add(models::batch_commit::Column::RevertedBlockNumber.is_null());

        // Fetch the batch hashes
        let batch_hashes = models::batch_commit::Entity::find()
            .select_only()
            .column(models::batch_commit::Column::Hash)
            .filter(filter.clone())
            .into_tuple::<Vec<u8>>()
            .all(self.get_connection())
            .await?;

        models::batch_commit::Entity::update_many()
            .filter(models::batch_commit::Column::Hash.is_in(batch_hashes.iter().cloned()))
            .col_expr(
                models::batch_commit::Column::RevertedBlockNumber,
                Expr::value(Some(block_info.number as i64)),
            )
            .col_expr(
                models::batch_commit::Column::Status,
                Expr::value(BatchStatus::Reverted.as_str()),
            )
            .exec(self.get_connection())
            .await?;

        models::l2_block::Entity::update_many()
            .filter(models::l2_block::Column::BatchHash.is_in(batch_hashes.iter().cloned()))
            .col_expr(models::l2_block::Column::Reverted, Expr::value(true))
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    async fn delete_batch_revert_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<u64, DatabaseError> {
        tracing::trace!(
            target: "scroll::db", block_number, "Deleting batch reverts greater than block number.");

        let batch_hashes = models::batch_commit::Entity::find()
            .select_only()
            .column(models::batch_commit::Column::Hash)
            .filter(models::batch_commit::Column::RevertedBlockNumber.gt(block_number as i64))
            .into_tuple::<Vec<u8>>()
            .all(self.get_connection())
            .await?;
        let num_batches = batch_hashes.len() as u64;

        models::batch_commit::Entity::update_many()
            .filter(models::batch_commit::Column::Hash.is_in(batch_hashes.iter().cloned()))
            .col_expr(models::batch_commit::Column::RevertedBlockNumber, Expr::value(None::<i64>))
            .col_expr(
                models::batch_commit::Column::Status,
                Expr::value(BatchStatus::Consolidated.as_str()),
            )
            .exec(self.get_connection())
            .await?;

        models::l2_block::Entity::update_many()
            .filter(models::l2_block::Column::BatchHash.is_in(batch_hashes.iter().cloned()))
            .col_expr(models::l2_block::Column::Reverted, Expr::value(false))
            .exec(self.get_connection())
            .await?;

        Ok(num_batches)
    }

    async fn change_batch_processing_to_committed_status(&self) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", "Changing batch status from processing to committed in database.");

        models::batch_commit::Entity::update_many()
            .filter(models::batch_commit::Column::Status.eq(BatchStatus::Processing.as_str()))
            .col_expr(
                models::batch_commit::Column::Status,
                Expr::value(BatchStatus::Committed.as_str()),
            )
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    async fn update_batch_status(
        &self,
        batch_hash: B256,
        status: BatchStatus,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", ?batch_hash, ?status, "Updating batch status in database.");

        models::batch_commit::Entity::update_many()
            .filter(models::batch_commit::Column::Hash.eq(batch_hash.to_vec()))
            .col_expr(models::batch_commit::Column::Status, Expr::value(status.as_str()))
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    async fn finalize_batches_up_to_index(
        &self,
        batch_index: u64,
        block_number: u64,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", ?batch_index, block_number, "Finalizing batch commits in database up to index.");

        models::batch_commit::Entity::update_many()
            .filter(
                models::batch_commit::Column::Index
                    .lte(batch_index)
                    .and(models::batch_commit::Column::FinalizedBlockNumber.is_null())
                    .and(models::batch_commit::Column::RevertedBlockNumber.is_null()),
            )
            .col_expr(
                models::batch_commit::Column::FinalizedBlockNumber,
                Expr::value(Some(block_number as i64)),
            )
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    async fn set_latest_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Updating the latest L1 block number in the database.");
        let metadata: models::metadata::ActiveModel =
            Metadata { key: "l1_latest_block".to_string(), value: block_number.to_string() }.into();
        Ok(models::metadata::Entity::insert(metadata)
            .on_conflict(
                OnConflict::column(models::metadata::Column::Key)
                    .update_column(models::metadata::Column::Value)
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    async fn set_finalized_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Updating the finalized L1 block number in the database.");

        // Remove all finalized L1 block infos less than or equal to the provided block number.
        self.remove_l1_block_info_leq(block_number).await?;

        // Insert or update the finalized L1 block number in metadata.
        let metadata: models::metadata::ActiveModel =
            Metadata { key: "l1_finalized_block".to_string(), value: block_number.to_string() }
                .into();
        Ok(models::metadata::Entity::insert(metadata)
            .on_conflict(
                OnConflict::column(models::metadata::Column::Key)
                    .update_column(models::metadata::Column::Value)
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    async fn set_processed_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Updating the processed L1 block number in the database.");
        let metadata: models::metadata::ActiveModel =
            Metadata { key: "l1_processed_block".to_string(), value: block_number.to_string() }
                .into();
        Ok(models::metadata::Entity::insert(metadata)
            .on_conflict(
                OnConflict::column(models::metadata::Column::Key)
                    .update_column(models::metadata::Column::Value)
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    async fn set_l2_head_block_number(&self, number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", ?number, "Updating the L2 head block number in the database.");
        let metadata: models::metadata::ActiveModel =
            Metadata { key: "l2_head_block".to_string(), value: number.to_string() }.into();
        Ok(models::metadata::Entity::insert(metadata)
            .on_conflict(
                OnConflict::column(models::metadata::Column::Key)
                    .update_column(models::metadata::Column::Value)
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    async fn finalize_consolidated_batches(
        &self,
        finalized_l1_block_number: u64,
    ) -> Result<Option<BlockInfo>, DatabaseError> {
        tracing::trace!(target: "scroll::db", finalized_l1_block_number, "Finalizing consolidated batches in the database.");
        let filter = Condition::all()
            .add(models::batch_commit::Column::FinalizedBlockNumber.is_not_null())
            .add(models::batch_commit::Column::FinalizedBlockNumber.lte(finalized_l1_block_number))
            .add(models::batch_commit::Column::Status.eq(BatchStatus::Consolidated.as_str()));
        let batch = models::batch_commit::Entity::find()
            .filter(filter.clone())
            .order_by_desc(models::batch_commit::Column::Index)
            .one(self.get_connection())
            .await?;

        if let Some(batch) = batch {
            let finalized_block_info = models::l2_block::Entity::find()
                .filter(models::l2_block::Column::BatchHash.eq(batch.hash.clone()))
                .order_by_desc(models::l2_block::Column::BlockNumber)
                .one(self.get_connection())
                .await?
                .map(|block| block.block_info())
                .expect("Finalized batch must have at least one L2 block.");
            models::batch_commit::Entity::update_many()
                .filter(filter)
                .col_expr(
                    models::batch_commit::Column::Status,
                    Expr::value(BatchStatus::Finalized.as_str()),
                )
                .exec(self.get_connection())
                .await?;

            Ok(Some(finalized_block_info))
        } else {
            Ok(None)
        }
    }

    async fn fetch_and_update_unprocessed_finalized_batches(
        &self,
        finalized_l1_block_number: u64,
    ) -> Result<Vec<BatchInfo>, DatabaseError> {
        let conditions = Condition::all()
            .add(models::batch_commit::Column::FinalizedBlockNumber.is_not_null())
            .add(models::batch_commit::Column::FinalizedBlockNumber.lte(finalized_l1_block_number))
            .add(models::batch_commit::Column::Status.eq(BatchStatus::Committed.as_str()));

        let batches = models::batch_commit::Entity::find()
            .filter(conditions.clone())
            .order_by_asc(models::batch_commit::Column::Index)
            .select_only()
            .column(models::batch_commit::Column::Index)
            .column(models::batch_commit::Column::Hash)
            .into_tuple::<(i64, Vec<u8>)>()
            .all(self.get_connection())
            .await
            .map(|x| {
                x.into_iter()
                    .map(|(index, hash)| BatchInfo::new(index as u64, B256::from_slice(&hash)))
                    .collect()
            })?;

        models::batch_commit::Entity::update_many()
            .col_expr(
                models::batch_commit::Column::Status,
                Expr::value(BatchStatus::Processing.as_str()),
            )
            .filter(conditions)
            .exec(self.get_connection())
            .await?;

        Ok(batches)
    }

    async fn fetch_and_update_unprocessed_committed_batches(
        &self,
    ) -> Result<Vec<BatchInfo>, DatabaseError> {
        let conditions = Condition::all()
            .add(models::batch_commit::Column::Status.eq(BatchStatus::Committed.as_str()));

        let batches = models::batch_commit::Entity::find()
            .filter(conditions.clone())
            .order_by_asc(models::batch_commit::Column::Index)
            .select_only()
            .column(models::batch_commit::Column::Index)
            .column(models::batch_commit::Column::Hash)
            .into_tuple::<(i64, Vec<u8>)>()
            .all(self.get_connection())
            .await
            .map(|x| {
                x.into_iter()
                    .map(|(index, hash)| BatchInfo::new(index as u64, B256::from_slice(&hash)))
                    .collect()
            })?;

        models::batch_commit::Entity::update_many()
            .col_expr(
                models::batch_commit::Column::Status,
                Expr::value(BatchStatus::Processing.as_str()),
            )
            .filter(conditions)
            .exec(self.get_connection())
            .await?;

        Ok(batches)
    }

    async fn delete_batches_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<u64, DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting batch inputs greater than block number.");
        Ok(models::batch_commit::Entity::delete_many()
            .filter(models::batch_commit::Column::BlockNumber.gt(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|x| x.rows_affected)?)
    }

    async fn delete_batches_gt_batch_index(&self, batch_index: u64) -> Result<u64, DatabaseError> {
        tracing::trace!(target: "scroll::db", batch_index, "Deleting batch inputs greater than batch index.");
        Ok(models::batch_commit::Entity::delete_many()
            .filter(models::batch_commit::Column::Index.gt(batch_index as i64))
            .exec(self.get_connection())
            .await
            .map(|x| x.rows_affected)?)
    }

    async fn insert_l1_message(&self, l1_message: L1MessageEnvelope) -> Result<(), DatabaseError> {
        let l1_index = l1_message.transaction.queue_index;
        tracing::trace!(target: "scroll::db", queue_index = l1_index, "Inserting L1 message into database.");

        let l1_message: models::l1_message::ActiveModel = l1_message.into();
        let result = models::l1_message::Entity::insert(l1_message)
            .on_conflict_do_nothing()
            .exec(self.get_connection())
            .await;

        if matches!(result, Err(DbErr::RecordNotInserted)) {
            tracing::error!(target: "scroll::db", queue_index = l1_index, "L1 message already exists");
            Ok(())
        } else {
            Ok(result.map(|_| ())?)
        }
    }

    async fn update_skipped_l1_messages(&self, indexes: Vec<u64>) -> Result<(), DatabaseError> {
        Ok(models::l1_message::Entity::update_many()
            .col_expr(models::l1_message::Column::Skipped, Expr::value(true))
            .filter(models::l1_message::Column::QueueIndex.is_in(indexes.iter().map(|&x| x as i64)))
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

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

    async fn prepare_l1_watcher_start_info(&self) -> Result<L1BlockStartupInfo, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Fetching startup safe block from database.");

        // set all batches with processing status back to committed
        self.change_batch_processing_to_committed_status().await?;

        // Get all L1 block infos from the database.
        let l1_block_infos = self.get_l1_block_info().await?;
        let latest_l1_block_info = self.get_latest_indexed_event_l1_block_number().await?;

        Ok(L1BlockStartupInfo::new(l1_block_infos, latest_l1_block_info))
    }

    async fn delete_l2_blocks_gt_block_number(
        &self,
        block_number: u64,
    ) -> Result<u64, DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Deleting L2 blocks greater than provided block number.");
        Ok(models::l2_block::Entity::delete_many()
            .filter(models::l2_block::Column::BlockNumber.gt(block_number as i64))
            .exec(self.get_connection())
            .await
            .map(|x| x.rows_affected)?)
    }

    async fn delete_l2_blocks_gt_batch_index(
        &self,
        batch_index: u64,
    ) -> Result<u64, DatabaseError> {
        tracing::trace!(target: "scroll::db", batch_index, "Deleting L2 blocks greater than provided batch index.");
        Ok(models::l2_block::Entity::delete_many()
            .filter(
                Condition::all()
                    .add(models::l2_block::Column::BatchIndex.is_not_null())
                    .add(models::l2_block::Column::BatchIndex.gt(batch_index as i64)),
            )
            .exec(self.get_connection())
            .await
            .map(|x| x.rows_affected)?)
    }

    async fn insert_blocks(
        &self,
        blocks: Vec<BlockInfo>,
        batch_info: BatchInfo,
    ) -> Result<(), DatabaseError> {
        // We only insert safe blocks into the database, we do not persist unsafe blocks.
        tracing::trace!(
            target: "scroll::db",
            batch_hash = ?batch_info.hash,
            batch_index = batch_info.index,
            blocks = ?blocks,
            "Inserting blocks into database."
        );
        let l2_blocks: Vec<models::l2_block::ActiveModel> =
            blocks.into_iter().map(|b| (b, batch_info).into()).collect();
        models::l2_block::Entity::insert_many(l2_blocks)
            .on_conflict(
                OnConflict::column(models::l2_block::Column::BlockHash)
                    .update_columns([
                        models::l2_block::Column::BlockNumber,
                        models::l2_block::Column::BatchHash,
                        models::l2_block::Column::BatchIndex,
                    ])
                    .to_owned(),
            )
            .on_empty_do_nothing()
            .exec_without_returning(self.get_connection())
            .await?;

        Ok(())
    }

    async fn insert_genesis_block(&self, genesis_hash: B256) -> Result<(), DatabaseError> {
        let genesis_block = BlockInfo::new(0, genesis_hash);
        let genesis_batch = BatchInfo::new(0, B256::ZERO);
        self.insert_blocks(vec![genesis_block], genesis_batch).await
    }

    async fn update_l1_messages_from_l2_blocks(
        &self,
        blocks: Vec<L2BlockInfoWithL1Messages>,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", num_blocks = blocks.len(), "Updating executed L1 messages from blocks with L2 block number in the database.");

        // First, purge all existing mappings for unsafe blocks.
        self.purge_l1_message_to_l2_block_mappings(blocks.first().map(|b| b.block_info.number))
            .await?;

        // Then, update the executed L1 messages for each block.
        self.update_l1_messages_with_l2_blocks(blocks).await?;

        Ok(())
    }

    async fn update_l1_messages_with_l2_blocks(
        &self,
        blocks: Vec<L2BlockInfoWithL1Messages>,
    ) -> Result<(), DatabaseError> {
        if blocks.is_empty() {
            return Ok(());
        }
        let start = blocks.first().unwrap().block_info.number;
        let end = blocks.last().unwrap().block_info.number;
        tracing::trace!(target: "scroll::db", start_block = start, end_block = end, "Updating executed L1 messages from blocks with L2 block number in the database.");

        let mut case = CaseStatement::new();
        let mut all_hashes = Vec::new();

        for block_info in blocks {
            if block_info.l1_messages.is_empty() {
                continue;
            }

            tracing::trace!(
                target: "scroll::db",
                block_number = block_info.block_info.number,
                l1_messages = ?block_info.l1_messages,
                "Including L1 messages from block in batch update."
            );

            let hashes: Vec<Vec<u8>> = block_info.l1_messages.iter().map(|x| x.to_vec()).collect();

            case = case.case(
                models::l1_message::Column::Hash.is_in(hashes.clone()),
                Expr::value(block_info.block_info.number as i64),
            );

            all_hashes.extend(hashes);
        }

        if all_hashes.is_empty() {
            return Ok(());
        }

        // query translates to the following sql:
        // UPDATE l1_message
        // SET l2_block_number = CASE
        //     WHEN hash IN (block1_hashes) THEN block1_number
        //     WHEN hash IN (block2_hashes) THEN block2_number
        //     WHEN hash IN (block3_hashes) THEN block3_number
        //     ELSE 0
        // END
        // WHERE hash IN (all_hashes)
        models::l1_message::Entity::update_many()
            .col_expr(models::l1_message::Column::L2BlockNumber, case.into())
            .filter(models::l1_message::Column::Hash.is_in(all_hashes))
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    async fn purge_l1_message_to_l2_block_mappings(
        &self,
        block_number: Option<u64>,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", ?block_number, "Purging L1 message to L2 block mappings from database.");

        let filter = if let Some(block_number) = block_number {
            models::l1_message::Column::L2BlockNumber.gte(block_number as i64)
        } else {
            let (safe_block_info, _batch_info) = self.get_latest_safe_l2_info().await?;
            models::l1_message::Column::L2BlockNumber.gt(safe_block_info.number as i64)
        };

        models::l1_message::Entity::update_many()
            .col_expr(models::l1_message::Column::L2BlockNumber, Expr::value(None::<i64>))
            .filter(filter)
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    async fn insert_batch_consolidation_outcome(
        &self,
        outcome: BatchConsolidationOutcome,
    ) -> Result<(), DatabaseError> {
        self.insert_blocks(
            outcome.blocks.iter().map(|b| b.block_info).collect(),
            outcome.batch_info,
        )
        .await?;
        self.update_l1_messages_with_l2_blocks(outcome.blocks).await?;
        self.update_skipped_l1_messages(outcome.skipped_l1_messages).await?;
        self.update_batch_status(outcome.batch_info.hash, outcome.target_status).await?;
        Ok(())
    }

    async fn unwind(&self, l1_block_number: u64) -> Result<UnwindResult, DatabaseError> {
        // Set the latest L1 block number
        self.set_latest_l1_block_number(l1_block_number).await?;

        // remove the L1 block infos greater than the provided l1 block number
        self.remove_l1_block_info_gt(l1_block_number).await?;

        // delete batch commits, l1 messages and batch finalization effects greater than the
        // provided l1 block number
        let batches_removed = self.delete_batches_gt_block_number(l1_block_number).await?;
        let deleted_messages = self.delete_l1_messages_gt(l1_block_number).await?;
        self.delete_batch_finalization_gt_block_number(l1_block_number).await?;
        let batch_reverts_removed: u64 =
            self.delete_batch_revert_gt_block_number(l1_block_number).await?;

        // filter and sort the executed L1 messages
        let mut removed_executed_l1_messages: Vec<_> =
            deleted_messages.into_iter().filter(|x| x.l2_block_number.is_some()).collect();
        removed_executed_l1_messages
            .sort_by(|a, b| a.transaction.queue_index.cmp(&b.transaction.queue_index));

        // check if we need to reorg the L2 head and delete some L2 blocks
        let (queue_index, l2_head_block_number) =
            if let Some(msg) = removed_executed_l1_messages.first() {
                let l2_reorg_block_number = msg
                    .l2_block_number
                    .expect("we guarantee that this is Some(u64) due to the filter above")
                    .saturating_sub(1);

                (Some(msg.transaction.queue_index), Some(l2_reorg_block_number))
            } else {
                (None, None)
            };

        // check if we need to reorg the L2 safe block
        let l2_safe_block_info = if batches_removed > 0 || batch_reverts_removed > 0 {
            let (safe_block_info, _batch_info) = self.get_latest_safe_l2_info().await?;
            Some(safe_block_info)
        } else {
            None
        };

        // delete mapping for l1 messages that were included in unsafe blocks after the reorg point
        if let Some(block_number) = l2_head_block_number {
            self.purge_l1_message_to_l2_block_mappings(Some(block_number.saturating_add(1)))
                .await?;
            self.set_l2_head_block_number(block_number).await?;
        }

        // commit the transaction
        Ok(UnwindResult { l1_block_number, queue_index, l2_head_block_number, l2_safe_block_info })
    }

    async fn insert_signatures(
        &self,
        signatures: Vec<(B256, Signature)>,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", num_signatures = signatures.len(), "Inserting block signatures into database.");

        if signatures.is_empty() {
            tracing::debug!(target: "scroll::db", "No signatures to insert.");
            return Ok(());
        }

        models::block_signature::Entity::insert_many(
            signatures
                .into_iter()
                .map(|(block_hash, signature)| (block_hash, signature).into())
                .collect::<Vec<models::block_signature::ActiveModel>>(),
        )
        .on_conflict(
            OnConflict::column(models::block_signature::Column::BlockHash)
                .update_column(models::block_signature::Column::Signature)
                .to_owned(),
        )
        .exec_without_returning(self.get_connection())
        .await?;

        Ok(())
    }

    async fn insert_signature(
        &self,
        block_hash: B256,
        signature: Signature,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_hash = ?block_hash, "Storing block signature in database.");

        let block_signature: models::block_signature::ActiveModel = (block_hash, signature).into();

        models::block_signature::Entity::insert(block_signature)
            .on_conflict(
                OnConflict::column(models::block_signature::Column::BlockHash)
                    .update_column(models::block_signature::Column::Signature)
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await
            .map(|_| ())?;

        Ok(())
    }
}

/// The [`DatabaseReadOperations`] trait provides read-only methods for interacting with the
/// database.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait DatabaseReadOperations {
    /// Get a [`BatchCommitData`] from the database by its batch index.
    async fn get_batch_by_index(
        &self,
        batch_index: u64,
    ) -> Result<Vec<BatchCommitData>, DatabaseError>;

    /// Get a [`BatchCommitData`] from the database by its batch hash.
    async fn get_batch_by_hash(
        &self,
        batch_hash: B256,
    ) -> Result<Option<BatchCommitData>, DatabaseError>;

    /// Get a [`BatchCommitData`] from the database by its batch hash.
    async fn get_batch_by_hash(
        &self,
        batch_hash: B256,
    ) -> Result<Option<BatchCommitData>, DatabaseError>;

    /// Get the status of a batch by its hash.
    #[cfg(test)]
    async fn get_batch_status_by_hash(
        &self,
        batch_hash: B256,
    ) -> Result<Option<rollup_node_primitives::BatchStatus>, DatabaseError>;

    /// Get all L1 block infos from the database ordered by block number ascending.
    async fn get_l1_block_info(&self) -> Result<Vec<BlockInfo>, DatabaseError>;

    /// Get the latest indexed event L1 block number from the database.
    async fn get_latest_indexed_event_l1_block_number(&self) -> Result<Option<u64>, DatabaseError>;

    /// Get the latest L1 block number from the database.
    async fn get_latest_l1_block_number(&self) -> Result<u64, DatabaseError>;

    /// Get the finalized L1 block number from the database.
    async fn get_finalized_l1_block_number(&self) -> Result<u64, DatabaseError>;

    /// Get the processed L1 block number from the database.
    async fn get_processed_l1_block_number(&self) -> Result<u64, DatabaseError>;

    /// Get the latest L2 head block info.
    async fn get_l2_head_block_number(&self) -> Result<u64, DatabaseError>;

    /// Get the L1 block number of the last batch commit in the database.
    async fn get_last_batch_commit_l1_block(&self) -> Result<Option<u64>, DatabaseError>;

    /// Get the L1 block number of the last L1 message in the database.
    async fn get_last_l1_message_l1_block(&self) -> Result<Option<u64>, DatabaseError>;

    /// Get a vector of n [`L1MessageEnvelope`]s in the database starting from the provided `start`
    /// point.
    async fn get_n_l1_messages(
        &self,
        start: Option<L1MessageKey>,
        n: usize,
    ) -> Result<Vec<L1MessageEnvelope>, DatabaseError>;

    /// Get the extra data for n block, starting at the provided block number.
    async fn get_n_l2_block_data_hint(
        &self,
        block_number: u64,
        n: usize,
    ) -> Result<Vec<BlockDataHint>, DatabaseError>;

    /// Get the [`BlockInfo`] and optional [`BatchInfo`] for the provided block hash.
    async fn get_l2_block_and_batch_info_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<(BlockInfo, BatchInfo)>, DatabaseError>;

    /// Get a [`BlockInfo`] from the database by its block number.
    async fn get_l2_block_info_by_number(
        &self,
        block_number: u64,
    ) -> Result<Option<BlockInfo>, DatabaseError>;

    /// Get the latest safe/finalized L2 ([`BlockInfo`], [`BatchInfo`]) from the database. Until we
    /// update the batch handling logic with issue #273, we don't differentiate between safe and
    /// finalized l2 blocks.
    async fn get_latest_safe_l2_info(&self) -> Result<(BlockInfo, BatchInfo), DatabaseError>;

    /// Returns the highest L2 block originating from the provided `batch_hash` or the highest block
    /// for the batch's index.
    async fn get_highest_block_for_batch_hash(
        &self,
        batch_hash: B256,
    ) -> Result<Option<BlockInfo>, DatabaseError>;

    /// Returns the highest L2 block originating from the provided `batch_index` or the highest
    /// block for the batch's index.
    async fn get_highest_block_for_batch_index(
        &self,
        batch_index: u64,
    ) -> Result<Option<BlockInfo>, DatabaseError>;

    /// Get a block signature from the database by block hash.
    /// TODO: remove this once we deprecated l2geth.
    async fn get_signature(&self, block_hash: B256) -> Result<Option<Signature>, DatabaseError>;
}

#[async_trait::async_trait]
impl<T: ReadConnectionProvider + Sync + ?Sized> DatabaseReadOperations for T {
    async fn get_batch_by_index(
        &self,
        batch_index: u64,
    ) -> Result<Vec<BatchCommitData>, DatabaseError> {
        Ok(models::batch_commit::Entity::find()
            .filter(
                models::batch_commit::Column::Index
                    .eq(TryInto::<i64>::try_into(batch_index).expect("index should fit in i64")),
            )
            .all(self.get_connection())
            .await
            .map(|x| x.into_iter().map(Into::into).collect())?)
    }

    async fn get_batch_by_hash(
        &self,
        batch_hash: B256,
    ) -> Result<Option<BatchCommitData>, DatabaseError> {
        Ok(models::batch_commit::Entity::find()
            .filter(
                models::batch_commit::Column::Index
                    .eq(TryInto::<i64>::try_into(batch_index).expect("index should fit in i64"))
                    .and(models::batch_commit::Column::RevertedBlockNumber.is_null()),
            )
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    async fn get_batch_by_hash(
        &self,
        batch_hash: B256,
    ) -> Result<Option<BatchCommitData>, DatabaseError> {
        Ok(models::batch_commit::Entity::find()
            .filter(models::batch_commit::Column::Hash.eq(batch_hash.to_vec()))
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    #[cfg(test)]
    async fn get_batch_status_by_hash(
        &self,
        batch_hash: B256,
    ) -> Result<Option<rollup_node_primitives::BatchStatus>, DatabaseError> {
        use std::str::FromStr;

        Ok(models::batch_commit::Entity::find()
            .filter(models::batch_commit::Column::Hash.eq(batch_hash.to_vec()))
            .select_only()
            .column(models::batch_commit::Column::Status)
            .into_tuple::<String>()
            .one(self.get_connection())
            .await?
            .map(|status_str| {
                rollup_node_primitives::BatchStatus::from_str(&status_str)
                    .expect("Invalid batch status in database")
            }))
    }

    async fn get_l1_block_info(&self) -> Result<Vec<BlockInfo>, DatabaseError> {
        let l1_blocks = models::l1_block::Entity::find()
            .order_by_asc(models::l1_block::Column::BlockNumber)
            .all(self.get_connection())
            .await?;

        Ok(l1_blocks.into_iter().map(Into::into).collect())
    }

    async fn get_latest_indexed_event_l1_block_number(&self) -> Result<Option<u64>, DatabaseError> {
        let latest_l1_message = models::l1_message::Entity::find()
            .select_only()
            .column_as(models::l1_message::Column::L1BlockNumber.max(), "max_l1_block_number")
            .into_tuple::<Option<i64>>()
            .one(self.get_connection())
            .await?
            .flatten();

        let latest_batch_event = models::batch_commit::Entity::find()
            .select_only()
            .filter(models::batch_commit::Column::Index.gt(0))
            .column_as(
                Expr::col(models::batch_commit::Column::BlockNumber).max(),
                "max_block_number",
            )
            .column_as(
                Expr::col(models::batch_commit::Column::FinalizedBlockNumber).max(),
                "max_finalized_block_number",
            )
            .column_as(
                Expr::col(models::batch_commit::Column::RevertedBlockNumber).max(),
                "max_reverted_block_number",
            )
            .into_tuple::<(Option<i64>, Option<i64>, Option<i64>)>()
            .one(self.get_connection())
            .await?
            .and_then(|tuple| <[Option<i64>; 3]>::from(tuple).into_iter().flatten().max());

        let latest_l1_block_number =
            [latest_l1_message, latest_batch_event].into_iter().flatten().max();

        Ok(latest_l1_block_number.map(|n| n as u64))
    }

    async fn get_latest_l1_block_number(&self) -> Result<u64, DatabaseError> {
        Ok(models::metadata::Entity::find()
            .filter(models::metadata::Column::Key.eq("l1_latest_block"))
            .select_only()
            .column(models::metadata::Column::Value)
            .into_tuple::<String>()
            .one(self.get_connection())
            .await?
            .expect("l1_latest_block should always be set")
            .parse::<u64>()
            .expect("l1_latest_block should always be a valid u64"))
    }

    async fn get_finalized_l1_block_number(&self) -> Result<u64, DatabaseError> {
        Ok(models::metadata::Entity::find()
            .filter(models::metadata::Column::Key.eq("l1_finalized_block"))
            .select_only()
            .column(models::metadata::Column::Value)
            .into_tuple::<String>()
            .one(self.get_connection())
            .await?
            .expect("l1_finalized_block should always be set")
            .parse::<u64>()
            .expect("l1_finalized_block should always be a valid u64"))
    }

    async fn get_processed_l1_block_number(&self) -> Result<u64, DatabaseError> {
        Ok(models::metadata::Entity::find()
            .filter(models::metadata::Column::Key.eq("l1_processed_block"))
            .select_only()
            .column(models::metadata::Column::Value)
            .into_tuple::<String>()
            .one(self.get_connection())
            .await?
            .expect("l1_processed_block should always be set")
            .parse::<u64>()
            .expect("l1_processed_block should always be a valid u64"))
    }

    async fn get_l2_head_block_number(&self) -> Result<u64, DatabaseError> {
        Ok(models::metadata::Entity::find()
            .filter(models::metadata::Column::Key.eq("l2_head_block"))
            .select_only()
            .column(models::metadata::Column::Value)
            .into_tuple::<String>()
            .one(self.get_connection())
            .await?
            .expect("l2_head_block should always be set")
            .parse::<u64>()
            .expect("l2_head_block should always be a valid u64"))
    }

    async fn get_last_batch_commit_l1_block(&self) -> Result<Option<u64>, DatabaseError> {
        Ok(models::batch_commit::Entity::find()
            .filter(models::batch_commit::Column::RevertedBlockNumber.is_null())
            .order_by_desc(models::batch_commit::Column::BlockNumber)
            .select_only()
            .column(models::batch_commit::Column::BlockNumber)
            .into_tuple::<i64>()
            .one(self.get_connection())
            .await?
            .map(|block_number| block_number as u64))
    }

    async fn get_last_l1_message_l1_block(&self) -> Result<Option<u64>, DatabaseError> {
        Ok(models::l1_message::Entity::find()
            .order_by_desc(models::l1_message::Column::L1BlockNumber)
            .select_only()
            .column(models::l1_message::Column::L1BlockNumber)
            .into_tuple::<i64>()
            .one(self.get_connection())
            .await?
            .map(|block_number| block_number as u64))
    }

    async fn get_n_l1_messages(
        &self,
        start: Option<L1MessageKey>,
        n: usize,
    ) -> Result<Vec<L1MessageEnvelope>, DatabaseError> {
        match start {
            // Provides n L1 messages with increasing queue index starting from the provided queue
            // index.
            Some(L1MessageKey::QueueIndex(queue_index)) => Ok(models::l1_message::Entity::find()
                .filter(models::l1_message::Column::QueueIndex.gte(queue_index))
                .order_by_asc(models::l1_message::Column::QueueIndex)
                .limit(Some(n as u64))
                .all(self.get_connection())
                .await
                .map(map_l1_message_result)?),
            // Provides n L1 messages with increasing queue index starting from the message with the
            // provided transaction hash.
            Some(L1MessageKey::TransactionHash(ref h)) => {
                // Lookup message by hash to get its queue index.
                let record = models::l1_message::Entity::find()
                    .filter(models::l1_message::Column::Hash.eq(h.to_vec()))
                    .one(self.get_connection())
                    .await?
                    .ok_or_else(|| {
                        DatabaseError::L1MessageNotFound(L1MessageKey::TransactionHash(*h))
                    })?;
                // Yield n messages starting from the found queue index.
                Ok(models::l1_message::Entity::find()
                    .filter(models::l1_message::Column::QueueIndex.gte(record.queue_index))
                    .order_by_asc(models::l1_message::Column::QueueIndex)
                    .limit(Some(n as u64))
                    .all(self.get_connection())
                    .await
                    .map(map_l1_message_result)?)
            }
            // Provides n L1 messages with increasing queue index starting from the message with
            // the provided queue hash.
            Some(L1MessageKey::QueueHash(ref h)) => {
                // Lookup message by queue hash.
                let record = models::l1_message::Entity::find()
                    .filter(
                        Condition::all()
                            .add(models::l1_message::Column::QueueHash.is_not_null())
                            .add(models::l1_message::Column::QueueHash.eq(h.to_vec())),
                    )
                    .one(self.get_connection())
                    .await?
                    .ok_or_else(|| DatabaseError::L1MessageNotFound(L1MessageKey::QueueHash(*h)))?;
                // Yield n messages starting from the found queue index.
                Ok(models::l1_message::Entity::find()
                    .filter(models::l1_message::Column::QueueIndex.gte(record.queue_index))
                    .order_by_asc(models::l1_message::Column::QueueIndex)
                    .limit(Some(n as u64))
                    .all(self.get_connection())
                    .await
                    .map(map_l1_message_result)?)
            }
            // Provides n L1 messages with increasing queue index starting from the message included
            // in the provided L2 block number.
            Some(L1MessageKey::BlockNumber(block_number)) => {
                // Lookup the latest message included in a block with a block number less than
                // the provided block number. This is achieved by filtering for messages with a
                // block number less than the provided block number and ordering by block number and
                // queue index in descending order. This ensures that we get the latest message
                // included in a block before the provided block number.
                if let Some(record) = models::l1_message::Entity::find()
                    .filter(models::l1_message::Column::L2BlockNumber.lt(block_number as i64))
                    .order_by_desc(models::l1_message::Column::L2BlockNumber)
                    .order_by_desc(models::l1_message::Column::QueueIndex)
                    .one(self.get_connection())
                    .await?
                {
                    // Yield n messages starting from the found queue index + 1. Only return
                    // messages that have not been skipped (skipped = false) to handle the edge
                    // case where the last message in a batch is skipped.
                    let condition = Condition::all()
                        // We add 1 to the queue index to constrain across block boundaries
                        .add(models::l1_message::Column::QueueIndex.gte(record.queue_index + 1))
                        .add(models::l1_message::Column::Skipped.eq(false));
                    Ok(models::l1_message::Entity::find()
                        .filter(condition)
                        .order_by_asc(models::l1_message::Column::QueueIndex)
                        .limit(Some(n as u64))
                        .all(self.get_connection())
                        .await
                        .map(map_l1_message_result)?)
                }
                // If no messages have been found then it suggests that no messages have been
                // included in blocks yet and as such we should n messages with increasing queue
                // index starting from the beginning.
                else {
                    Ok(models::l1_message::Entity::find()
                        .filter(models::l1_message::Column::Skipped.eq(false))
                        .order_by_asc(models::l1_message::Column::QueueIndex)
                        .limit(Some(n as u64))
                        .all(self.get_connection())
                        .await
                        .map(map_l1_message_result)?)
                }
            }
            // Provides a stream over all L1 messages with increasing queue index starting that have
            // not been included in an L2 block and have a block number less than or equal to the
            // finalized L1 block number (they have been finalized on L1).
            Some(L1MessageKey::NotIncluded(NotIncludedKey::FinalizedWithBlockDepth(depth))) => {
                // Lookup the finalized L1 block number.
                let finalized_block_number = self.get_finalized_l1_block_number().await?;

                // Calculate the target block number by subtracting the depth from the finalized
                // block number. If the depth is greater than the finalized block number, we return
                // an empty vector as there are no messages that satisfy the condition.
                let target_block_number =
                    if let Some(target_block_number) = finalized_block_number.checked_sub(depth) {
                        target_block_number
                    } else {
                        return Ok(vec![]);
                    };

                // Create a filter condition for messages that have an L1 block number less than or
                // equal to the finalized block number and have not been included in an L2 block
                // (i.e. L2BlockNumber is null) nor skipped.
                let condition = Condition::all()
                    .add(models::l1_message::Column::L1BlockNumber.lte(target_block_number as i64))
                    .add(models::l1_message::Column::L2BlockNumber.is_null())
                    .add(models::l1_message::Column::Skipped.eq(false));
                // Yield n messages matching the condition ordered by increasing queue index.
                Ok(models::l1_message::Entity::find()
                    .filter(condition)
                    .order_by_asc(models::l1_message::Column::QueueIndex)
                    .limit(Some(n as u64))
                    .all(self.get_connection())
                    .await
                    .map(map_l1_message_result)?)
            }
            // Provides N L1 messages with increasing queue index starting that have not been
            // included in an L2 block and have a block number less than or equal to the
            // latest L1 block number minus the provided depth (they have been sufficiently deep
            // on L1 to be considered safe to include - reorg risk is low).
            Some(L1MessageKey::NotIncluded(NotIncludedKey::BlockDepth(depth))) => {
                // Lookup the latest L1 block number.
                let latest_block_number = self.get_latest_l1_block_number().await?;

                // Calculate the target block number by subtracting the depth from the latest block
                // number. If the depth is greater than the latest block number, we return an empty
                // vector as there are no messages that satisfy the condition.
                let target_block_number =
                    if let Some(target_block_number) = latest_block_number.checked_sub(depth) {
                        target_block_number
                    } else {
                        return Ok(vec![]);
                    };
                // Create a filter condition for messages that have an L1 block number less than
                // or equal to the target block number and have not been included in an L2 block
                // (i.e. L2BlockNumber is null) nor skipped.
                let condition = Condition::all()
                    .add(models::l1_message::Column::L1BlockNumber.lte(target_block_number as i64))
                    .add(models::l1_message::Column::Skipped.eq(false))
                    .add(models::l1_message::Column::L2BlockNumber.is_null());
                // Yield n messages matching the condition ordered by increasing queue index.
                Ok(models::l1_message::Entity::find()
                    .filter(condition)
                    .order_by_asc(models::l1_message::Column::QueueIndex)
                    .limit(Some(n as u64))
                    .all(self.get_connection())
                    .await
                    .map(map_l1_message_result)?)
            }
            // Provides n L1 messages with increasing queue index starting from the beginning.
            None => Ok(models::l1_message::Entity::find()
                .order_by_asc(models::l1_message::Column::QueueIndex)
                .limit(Some(n as u64))
                .all(self.get_connection())
                .await
                .map(map_l1_message_result)?),
        }
    }

    async fn get_n_l2_block_data_hint(
        &self,
        block_number: u64,
        n: usize,
    ) -> Result<Vec<BlockDataHint>, DatabaseError> {
        Ok(models::block_data::Entity::find()
            .filter(models::block_data::Column::Number.gte(block_number as i64))
            .limit(Some(n as u64))
            .all(self.get_connection())
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    async fn get_l2_block_and_batch_info_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<(BlockInfo, BatchInfo)>, DatabaseError> {
        tracing::trace!(target: "scroll::db", ?block_hash, "Fetching L2 block and batch info by hash from database.");
        Ok(models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BlockHash.eq(block_hash.to_vec()))
            .one(self.get_connection())
            .await
            .map(|x| {
                x.map(|x| {
                    let (block_info, batch_info): (BlockInfo, BatchInfo) = x.into();
                    (block_info, batch_info)
                })
            })?)
    }

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
                    let (block_info, _maybe_batch_info): (BlockInfo, BatchInfo) = x.into();
                    block_info
                })
            })?)
    }

    async fn get_latest_safe_l2_info(&self) -> Result<(BlockInfo, BatchInfo), DatabaseError> {
        tracing::trace!(target: "scroll::db", "Fetching latest safe L2 block from database.");
        let filter = Condition::all()
            .add(models::l2_block::Column::BatchIndex.is_not_null())
            .add(models::l2_block::Column::Reverted.eq(false));

        let safe_block = models::l2_block::Entity::find()
            .filter(filter)
            .order_by_desc(models::l2_block::Column::BlockNumber)
            .one(self.get_connection())
            .await?
            .expect("there should always be at least the genesis block in the database");

        Ok((safe_block.block_info(), safe_block.batch_info()))
    }

    async fn get_highest_block_for_batch_hash(
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

    async fn get_highest_block_for_batch_index(
        &self,
        batch_index: u64,
    ) -> Result<Option<BlockInfo>, DatabaseError> {
        Ok(models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BatchIndex.lte(batch_index))
            .order_by_desc(models::l2_block::Column::BlockNumber)
            .one(self.get_connection())
            .await?
            .map(|model| model.block_info()))
    }

    async fn get_signature(&self, block_hash: B256) -> Result<Option<Signature>, DatabaseError> {
        tracing::trace!(target: "scroll::db", block_hash = ?block_hash, "Retrieving block signature from database.");

        let block_signature = models::block_signature::Entity::find_by_id(block_hash.to_vec())
            .one(self.get_connection())
            .await?;

        match block_signature {
            Some(model) => {
                let signature = model.get_signature().map_err(|e| {
                    DatabaseError::ParseSignatureError(format!("Failed to parse signature: {}", e))
                })?;
                Ok(Some(signature))
            }
            None => Ok(None),
        }
    }
}

/// A key for an L1 message stored in the database.
///
/// It can either be the queue index, queue hash or the transaction hash.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum L1MessageKey {
    /// The queue index of the message.
    QueueIndex(u64),
    /// The queue hash of the message.
    QueueHash(B256),
    /// The transaction hash of the message.
    TransactionHash(B256),
    /// Start from the first message for the provided block number.
    BlockNumber(u64),
    /// Start from messages that have not been included in a block yet.
    NotIncluded(NotIncludedKey),
}

impl L1MessageKey {
    /// Create a new [`L1MessageKey`] from a queue index.
    pub const fn from_queue_index(index: u64) -> Self {
        Self::QueueIndex(index)
    }

    /// Create a new [`L1MessageKey`] from a queue hash.
    pub const fn from_queue_hash(hash: B256) -> Self {
        Self::QueueHash(hash)
    }

    /// Create a new [`L1MessageKey`] from a transaction hash.
    pub const fn from_transaction_hash(hash: B256) -> Self {
        Self::TransactionHash(hash)
    }

    /// Creates a new [`L1MessageKey`] for the provided block number.
    pub const fn block_number(number: u64) -> Self {
        Self::BlockNumber(number)
    }
}

/// This type defines where to start when fetching L1 messages that have not been included in a
/// block yet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NotIncludedKey {
    /// Start from finalized messages that have not been included in a block yet and have a L1
    /// block number that is a specified number of blocks below the current finalized L1 block
    /// number.
    #[serde(rename = "notIncludedFinalizedWithBlockDepth")]
    FinalizedWithBlockDepth(u64),
    /// Start from unfinalized messages that are included in L1 blocks at a specific depth.
    #[serde(rename = "notIncludedBlockDepth")]
    BlockDepth(u64),
}

fn map_l1_message_result(res: Vec<models::l1_message::Model>) -> Vec<L1MessageEnvelope> {
    res.into_iter().map(Into::into).collect()
}

impl fmt::Display for L1MessageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueIndex(index) => write!(f, "QueueIndex({index})"),
            Self::QueueHash(hash) => write!(f, "QueueHash({hash:#x})"),
            Self::TransactionHash(hash) => write!(f, "TransactionHash({hash:#x})"),
            Self::BlockNumber(number) => write!(f, "BlockNumber({number})"),
            Self::NotIncluded(start) => match start {
                NotIncludedKey::FinalizedWithBlockDepth(depth) => {
                    write!(f, "NotIncluded(Finalized:{depth})")
                }
                NotIncludedKey::BlockDepth(depth) => {
                    write!(f, "NotIncluded(BlockDepth({depth}))")
                }
            },
        }
    }
}

/// The result of [`DatabaseWriteOperations::unwind`].
#[derive(Debug)]
pub struct UnwindResult {
    /// The L1 block number that we unwinded to.
    pub l1_block_number: u64,
    /// The latest unconsumed queue index after the uwnind.
    pub queue_index: Option<u64>,
    /// The L2 head block number after the unwind. This is only populated if the L2 head has
    /// reorged.
    pub l2_head_block_number: Option<u64>,
    /// The L2 safe block info after the unwind. This is only populated if the L2 safe has reorged.
    pub l2_safe_block_info: Option<BlockInfo>,
}

mod tests {

    #[test]
    fn test_l1_message_key_serialization() {
        use crate::{L1MessageKey, NotIncludedKey};
        use alloy_primitives::B256;
        use std::str::FromStr;

        // Test for `L1MessageKey::QueueIndex`
        let key = L1MessageKey::QueueIndex(42);
        let json = serde_json::to_string(&key).unwrap();
        let decoded: L1MessageKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key, decoded);

        // Test for `L1MessageKey::TransactionHash`
        let key = L1MessageKey::TransactionHash(
            B256::from_str("0xa46f0b1dbe17b3d0d86fa70cef4a23dca5efcd35858998cc8c53140d01429746")
                .unwrap(),
        );
        let json = serde_json::to_string(&key).unwrap();
        let decoded: L1MessageKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key, decoded);

        // Test for `L1MessageKey::NotIncluded`
        let key = L1MessageKey::NotIncluded(NotIncludedKey::FinalizedWithBlockDepth(100));
        let json = serde_json::to_string(&key).unwrap();
        let decoded: L1MessageKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key, decoded);
    }

    #[test]
    fn test_l1_message_key_manual_serialization() {
        use crate::{L1MessageKey, NotIncludedKey};
        use alloy_primitives::B256;
        use std::str::FromStr;

        // Test for `L1MessageKey::QueueIndex`
        let json_string_queue_index = r#"{"queueIndex":42}"#;
        let decoded_queue_index: L1MessageKey =
            serde_json::from_str(json_string_queue_index).unwrap();
        assert_eq!(decoded_queue_index, L1MessageKey::QueueIndex(42));

        // Test for `L1MessageKey::TransactionHash`
        let json_string_transaction_hash = r#"{"transactionHash":"0xa46f0b1dbe17b3d0d86fa70cef4a23dca5efcd35858998cc8c53140d01429746"}"#;
        let decoded_transaction_hash: L1MessageKey =
            serde_json::from_str(json_string_transaction_hash).unwrap();
        assert_eq!(
            decoded_transaction_hash,
            L1MessageKey::TransactionHash(
                B256::from_str(
                    "0xa46f0b1dbe17b3d0d86fa70cef4a23dca5efcd35858998cc8c53140d01429746"
                )
                .unwrap()
            )
        );

        // Test for `L1MessageKey::NotIncluded`
        let json_string_not_included_key =
            r#"{"notIncluded":{"notIncludedFinalizedWithBlockDepth":100}}"#;
        let decoded_not_included_key: L1MessageKey =
            serde_json::from_str(json_string_not_included_key).unwrap();
        assert_eq!(
            decoded_not_included_key,
            L1MessageKey::NotIncluded(NotIncludedKey::FinalizedWithBlockDepth(100))
        );
    }
}
