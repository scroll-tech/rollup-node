use super::{models, DatabaseError};
use crate::{ReadConnectionProvider, WriteConnectionProvider};

use alloy_primitives::{Signature, B256};
use futures::{Stream, StreamExt};
use rollup_node_primitives::{
    BatchCommitData, BatchConsolidationOutcome, BatchInfo, BlockConsolidationOutcome, BlockInfo,
    L1MessageEnvelope, L2BlockInfoWithL1Messages, Metadata,
};
use scroll_alloy_rpc_types_engine::BlockDataHint;
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ColumnTrait, Condition, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use std::fmt;

/// The [`DatabaseWriteOperations`] trait provides write methods for interacting with the
/// database.
#[async_trait::async_trait]
pub trait DatabaseWriteOperations: WriteConnectionProvider + DatabaseReadOperations {
    /// Insert a [`BatchCommitData`] into the database.
    async fn insert_batch(&self, batch_commit: BatchCommitData) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", batch_hash = ?batch_commit.hash, batch_index = batch_commit.index, "Inserting batch input into database.");
        let batch_commit: models::batch_commit::ActiveModel = batch_commit.into();
        Ok(models::batch_commit::Entity::insert(batch_commit)
            .on_conflict(
                OnConflict::column(models::batch_commit::Column::Index)
                    .update_columns(vec![
                        models::batch_commit::Column::Hash,
                        models::batch_commit::Column::BlockNumber,
                        models::batch_commit::Column::BlockTimestamp,
                        models::batch_commit::Column::Calldata,
                        models::batch_commit::Column::BlobHash,
                        models::batch_commit::Column::FinalizedBlockNumber,
                    ])
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await
            .map(|_| ())?)
    }

    /// Finalizes all [`BatchCommitData`] up to the provided `batch_index` by setting their
    /// finalized block number to the provided block number.
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
                    .and(models::batch_commit::Column::FinalizedBlockNumber.is_null()),
            )
            .col_expr(
                models::batch_commit::Column::FinalizedBlockNumber,
                Expr::value(Some(block_number as i64)),
            )
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    /// Set the finalized L1 block number.
    async fn set_finalized_l1_block_number(&self, block_number: u64) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", block_number, "Updating the finalized L1 block number in the database.");
        let metadata: models::metadata::ActiveModel =
            Metadata { l1_finalized_block: block_number }.into();
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

    /// Fetches unprocessed batches up to the provided finalized L1 block number and updates their
    /// status.
    async fn fetch_and_update_unprocessed_finalized_batches(
        &self,
        finalized_l1_block_number: u64,
    ) -> Result<Vec<BatchInfo>, DatabaseError> {
        let conditions = Condition::all()
            .add(models::batch_commit::Column::FinalizedBlockNumber.is_not_null())
            .add(models::batch_commit::Column::FinalizedBlockNumber.lte(finalized_l1_block_number))
            .add(models::batch_commit::Column::Processed.eq(false));

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
            .col_expr(models::batch_commit::Column::Processed, Expr::value(true))
            .filter(conditions)
            .exec(self.get_connection())
            .await?;

        Ok(batches)
    }

    /// Delete all [`BatchCommitData`]s with a block number greater than the provided block number.
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

    /// Delete all [`BatchCommitData`]s with a batch index greater than the provided index.
    async fn delete_batches_gt_batch_index(&self, batch_index: u64) -> Result<u64, DatabaseError> {
        tracing::trace!(target: "scroll::db", batch_index, "Deleting batch inputs greater than batch index.");
        Ok(models::batch_commit::Entity::delete_many()
            .filter(models::batch_commit::Column::Index.gt(batch_index as i64))
            .exec(self.get_connection())
            .await
            .map(|x| x.rows_affected)?)
    }

    /// Insert an [`L1MessageEnvelope`] into the database.
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

    /// Prepare the database on startup and return metadata used for other components in the
    /// rollup-node.
    ///
    /// This method first unwinds the database to the finalized L1 block. It then fetches the batch
    /// info for the latest safe L2 block. It takes note of the L1 block number at which
    /// this batch was produced (currently the finalized block for the batch until we implement
    /// issue #273). It then retrieves the latest block for the previous batch (i.e., the batch
    /// before the latest safe block). It returns a tuple of this latest fetched block and the
    /// L1 block number of the batch.
    async fn prepare_on_startup(
        &self,
        genesis_hash: B256,
    ) -> Result<(Option<BlockInfo>, Option<u64>), DatabaseError> {
        tracing::trace!(target: "scroll::db", "Fetching startup safe block from database.");

        // Unwind the database to the last finalized L1 block saved in database.
        let finalized_block_number = self.get_finalized_l1_block_number().await?.unwrap_or(0);
        self.unwind(genesis_hash, finalized_block_number).await?;

        // Delete all unprocessed batches from the database and return starting l2 safe head and l1
        // head.
        if let Some(batch_info) = self
            .get_latest_safe_l2_info()
            .await?
            .map(|(_, batch_info)| batch_info)
            .filter(|b| b.index > 1)
        {
            let batch = self.get_batch_by_index(batch_info.index).await?.expect("batch must exist");
            self.delete_batches_gt_block_number(batch.block_number.saturating_sub(1)).await?;
        };

        let Some((block_info, batch_info)) =
            self.get_latest_safe_l2_info().await?.filter(|(block_info, _)| block_info.number > 0)
        else {
            return Ok((None, None))
        };
        let batch = self.get_batch_by_index(batch_info.index).await?.expect("batch must exist");
        Ok((Some(block_info), Some(batch.block_number.saturating_add(1))))
    }

    /// Delete all L2 blocks with a block number greater than the provided block number.
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

    /// Delete all L2 blocks with a batch index greater than the batch index.
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

    /// Insert multiple blocks into the database.
    async fn insert_blocks(
        &self,
        blocks: Vec<BlockInfo>,
        batch_info: BatchInfo,
    ) -> Result<(), DatabaseError> {
        for block in blocks {
            self.insert_block(block, batch_info).await?;
        }
        Ok(())
    }

    /// Insert a new block in the database.
    async fn insert_block(
        &self,
        block_info: BlockInfo,
        batch_info: BatchInfo,
    ) -> Result<(), DatabaseError> {
        // We only insert safe blocks into the database, we do not persist unsafe blocks.
        tracing::trace!(
            target: "scroll::db",
            batch_hash = ?batch_info.hash,
            batch_index = batch_info.index,
            block_number = block_info.number,
            block_hash = ?block_info.hash,
            "Inserting block into database."
        );
        let l2_block: models::l2_block::ActiveModel = (block_info, batch_info).into();
        models::l2_block::Entity::insert(l2_block)
            .on_conflict(
                OnConflict::column(models::l2_block::Column::BlockNumber)
                    .update_columns([
                        models::l2_block::Column::BlockHash,
                        models::l2_block::Column::BatchHash,
                        models::l2_block::Column::BatchIndex,
                    ])
                    .to_owned(),
            )
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    /// Insert the genesis block into the database.
    async fn insert_genesis_block(&self, genesis_hash: B256) -> Result<(), DatabaseError> {
        let genesis_block = BlockInfo::new(0, genesis_hash);
        let genesis_batch = BatchInfo::new(0, B256::ZERO);
        self.insert_block(genesis_block, genesis_batch).await
    }

    /// Update the executed L1 messages from the provided L2 blocks in the database.
    async fn update_l1_messages_from_l2_blocks(
        &self,
        blocks: Vec<L2BlockInfoWithL1Messages>,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", num_blocks = blocks.len(), "Updating executed L1 messages from blocks with L2 block number in the database.");

        // First, purge all existing mappings for unsafe blocks.
        self.purge_l1_message_to_l2_block_mappings(blocks.first().map(|b| b.block_info.number))
            .await?;

        // Then, update the executed L1 messages for each block.
        for block in blocks {
            self.update_l1_messages_with_l2_block(block).await?;
        }
        Ok(())
    }

    /// Update the executed L1 messages with the provided L2 block number in the database.
    async fn update_l1_messages_with_l2_block(
        &self,
        block_info: L2BlockInfoWithL1Messages,
    ) -> Result<(), DatabaseError> {
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

    /// Purge all L1 message to L2 block mappings from the database for blocks greater or equal to
    /// the provided block number. If the no block number is provided, purge mappings for all
    /// unsafe blocks.
    async fn purge_l1_message_to_l2_block_mappings(
        &self,
        block_number: Option<u64>,
    ) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", ?block_number, "Purging L1 message to L2 block mappings from database.");

        let filter = if let Some(block_number) = block_number {
            models::l1_message::Column::L2BlockNumber.gte(block_number as i64)
        } else {
            let safe_block_number = self.get_latest_safe_l2_info().await?;
            models::l1_message::Column::L2BlockNumber
                .gt(safe_block_number.map(|(block_info, _)| block_info.number as i64).unwrap_or(0))
        };

        models::l1_message::Entity::update_many()
            .col_expr(models::l1_message::Column::L2BlockNumber, Expr::value(None::<i64>))
            .filter(filter)
            .exec(self.get_connection())
            .await?;

        Ok(())
    }

    /// Insert the outcome of a batch consolidation into the database.
    async fn insert_batch_consolidation_outcome(
        &self,
        outcome: BatchConsolidationOutcome,
    ) -> Result<(), DatabaseError> {
        for block in outcome.blocks {
            match block {
                BlockConsolidationOutcome::Consolidated(block_info) => {
                    self.insert_block(block_info, outcome.batch_info).await?;
                }
                BlockConsolidationOutcome::Reorged(block_info) => {
                    self.insert_block(block_info.block_info, outcome.batch_info).await?;
                    self.update_l1_messages_with_l2_block(block_info).await?;
                }
            }
        }
        Ok(())
    }

    /// Unwinds the chain orchestrator by deleting all indexed data greater than the provided L1
    /// block number.
    async fn unwind(
        &self,
        genesis_hash: B256,
        l1_block_number: u64,
    ) -> Result<UnwindResult, DatabaseError> {
        // delete batch inputs and l1 messages
        let batches_removed = self.delete_batches_gt_block_number(l1_block_number).await?;
        let deleted_messages = self.delete_l1_messages_gt(l1_block_number).await?;

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
        let l2_safe_block_info = if batches_removed > 0 {
            if let Some(x) = self.get_latest_safe_l2_info().await? {
                Some(x.0)
            } else {
                Some(BlockInfo::new(0, genesis_hash))
            }
        } else {
            None
        };

        // commit the transaction
        Ok(UnwindResult { l1_block_number, queue_index, l2_head_block_number, l2_safe_block_info })
    }

    /// Store a block signature in the database.
    /// TODO: remove this once we deprecated l2geth.
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
pub trait DatabaseReadOperations: ReadConnectionProvider + Sync {
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

    /// Get the finalized L1 block number from the database.
    async fn get_finalized_l1_block_number(&self) -> Result<Option<u64>, DatabaseError> {
        Ok(models::metadata::Entity::find()
            .filter(models::metadata::Column::Key.eq("l1_finalized_block"))
            .select_only()
            .column(models::metadata::Column::Value)
            .into_tuple::<String>()
            .one(self.get_connection())
            .await
            .map(|x| x.and_then(|x| x.parse::<u64>().ok()))?)
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

    /// Gets the latest L1 messages which has an associated L2 block number if any.
    async fn get_latest_executed_l1_message(
        &self,
    ) -> Result<Option<L1MessageEnvelope>, DatabaseError> {
        Ok(models::l1_message::Entity::find()
            .filter(models::l1_message::Column::L2BlockNumber.is_not_null())
            .order_by_desc(models::l1_message::Column::L2BlockNumber)
            .order_by_desc(models::l1_message::Column::QueueIndex)
            .one(self.get_connection())
            .await?
            .map(Into::into))
    }

    /// Get an iterator over all [`L1MessageEnvelope`]s in the database starting from the provided
    /// `start` point.
    async fn get_l1_messages<'a>(
        &'a self,
        start: Option<L1MessageStart>,
    ) -> Result<
        Option<impl Stream<Item = Result<L1MessageEnvelope, DatabaseError>> + 'a>,
        DatabaseError,
    > {
        let queue_index = match start {
            Some(L1MessageStart::Index(i)) => Ok::<_, DatabaseError>(Some(i)),
            Some(L1MessageStart::Hash(ref h)) => {
                // Lookup message by hash
                let record = models::l1_message::Entity::find()
                    .filter(models::l1_message::Column::Hash.eq(h.to_vec()))
                    .one(self.get_connection())
                    .await?
                    .ok_or_else(|| DatabaseError::L1MessageNotFound(L1MessageStart::Hash(*h)))?;

                Ok(Some(record.queue_index as u64))
            }
            Some(L1MessageStart::BlockNumber(block_number)) => {
                let exact_match = models::l1_message::Entity::find()
                    .filter(models::l1_message::Column::L1BlockNumber.eq(block_number as i64))
                    .order_by_desc(models::l1_message::Column::QueueIndex)
                    .into_tuple::<i64>()
                    .one(self.get_connection())
                    .await?;

                if let Some(queue_index) = exact_match {
                    Ok(Some(queue_index as u64))
                } else {
                    // If no exact match is found, find the last message before the block number
                    Ok(models::l1_message::Entity::find()
                        .filter(models::l1_message::Column::L1BlockNumber.lt(block_number as i64))
                        .order_by_desc(models::l1_message::Column::L1BlockNumber)
                        .order_by_desc(models::l1_message::Column::QueueIndex)
                        .into_tuple::<i64>()
                        .one(self.get_connection())
                        .await?
                        .map(|x| x as u64))
                }
            }
            Some(L1MessageStart::NotIncluded(NotIncludedStart::Finalized)) => {
                let finalized_block_number = self
                    .get_finalized_l1_block_number()
                    .await?
                    .ok_or(DatabaseError::FinalizedL1BlockNotFound)?;
                let condition = Condition::all()
                    .add(
                        models::l1_message::Column::L1BlockNumber
                            .lte(finalized_block_number as i64),
                    )
                    .add(models::l1_message::Column::L2BlockNumber.is_null());
                Ok(models::l1_message::Entity::find()
                    .filter(condition)
                    .order_by_desc(models::l1_message::Column::QueueIndex)
                    .into_tuple::<i64>()
                    .one(self.get_connection())
                    .await?
                    .map(|x| x as u64))
            }
            Some(L1MessageStart::NotIncluded(NotIncludedStart::BlockDepth(depth))) => {
                // TODO: USE LATEST BLOCK NUMBER NOT FINALIZED
                let finalized_block_number = self
                    .get_finalized_l1_block_number()
                    .await?
                    .ok_or(DatabaseError::FinalizedL1BlockNotFound)?;
                let target_block_number = finalized_block_number.checked_sub(depth);
                if let Some(target_block_number) = target_block_number {
                    let condition = Condition::all()
                        .add(
                            models::l1_message::Column::L1BlockNumber
                                .lte(target_block_number as i64),
                        )
                        .add(models::l1_message::Column::L2BlockNumber.is_null());
                    Ok(models::l1_message::Entity::find()
                        .filter(condition)
                        .order_by_desc(models::l1_message::Column::QueueIndex)
                        .into_tuple::<i64>()
                        .one(self.get_connection())
                        .await?
                        .map(|x| x as u64))
                } else {
                    Ok(None)
                }
            }
            None => Ok(Some(0)),
        }?;

        let queue_index =
            if let Some(queue_index) = queue_index { queue_index } else { return Ok(None) };

        Ok(Some(
            models::l1_message::Entity::find()
                .filter(models::l1_message::Column::QueueIndex.gte(queue_index))
                .order_by_asc(models::l1_message::Column::QueueIndex)
                .stream(self.get_connection())
                .await?
                .map(|res| Ok(res.map(Into::into)?)),
        ))
    }

    /// Get the extra data for the provided block number.
    async fn get_l2_block_data_hint(
        &self,
        block_number: u64,
    ) -> Result<Option<BlockDataHint>, DatabaseError> {
        Ok(models::block_data::Entity::find()
            .filter(models::block_data::Column::Number.eq(block_number as i64))
            .one(self.get_connection())
            .await
            .map(|x| x.map(Into::into))?)
    }

    /// Get the [`BlockInfo`] and optional [`BatchInfo`] for the provided block hash.
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
                    let (block_info, _maybe_batch_info): (BlockInfo, BatchInfo) = x.into();
                    block_info
                })
            })?)
    }

    /// Get the latest safe L2 ([`BlockInfo`], [`BatchInfo`]) from the database.
    async fn get_latest_safe_l2_info(
        &self,
    ) -> Result<Option<(BlockInfo, BatchInfo)>, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Fetching latest safe L2 block from database.");
        Ok(models::l2_block::Entity::find()
            .filter(models::l2_block::Column::BatchIndex.is_not_null())
            .order_by_desc(models::l2_block::Column::BlockNumber)
            .one(self.get_connection())
            .await
            .map(|x| x.map(|x| (x.block_info(), x.batch_info())))?)
    }

    /// Get an iterator over all L2 blocks in the database starting from the most recent one.
    async fn get_l2_blocks<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<BlockInfo, DatabaseError>> + 'a, DatabaseError> {
        tracing::trace!(target: "scroll::db", "Fetching L2 blocks from database.");
        Ok(models::l2_block::Entity::find()
            .order_by_desc(models::l2_block::Column::BlockNumber)
            .stream(self.get_connection())
            .await?
            .map(|res| Ok(res.map(|res| res.block_info())?)))
    }

    /// Returns the highest L2 block originating from the provided `batch_hash` or the highest block
    /// for the batch's index.
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

    /// Returns the highest L2 block originating from the provided `batch_index` or the highest
    /// block for the batch's index.
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

    /// Get a block signature from the database by block hash.
    /// TODO: remove this once we deprecated l2geth.
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

/// This type defines the start of an L1 message stream.
///
/// It can either be an index, which is the queue index of the first message to return, or a hash,
/// which is the hash of the first message to return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum L1MessageStart {
    /// Start from the provided queue index.
    Index(u64),
    /// Start from the provided queue hash.
    Hash(B256),
    /// Start from the first message for the provided block number.
    BlockNumber(u64),
    /// Start from messages that have not been included in a block yet.
    NotIncluded(NotIncludedStart),
}

/// This type defines where to start when fetching L1 messages that have not been included in a
/// block yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotIncludedStart {
    /// Start from finalized messages that have not been included in a block yet.
    Finalized,
    /// Start from unfinalized messages that are included in L1 blocks at a specific depth.
    BlockDepth(u64),
}

impl L1MessageStart {
    /// Creates a new [`L1MessageStart`] for the provided queue index.
    pub fn index(index: u64) -> Self {
        Self::Index(index)
    }

    /// Creates a new [`L1MessageStart`] for the provided block number.
    pub fn block_number(number: u64) -> Self {
        Self::BlockNumber(number)
    }
}

impl fmt::Display for L1MessageStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index(index) => write!(f, "Index({index})"),
            Self::Hash(hash) => write!(f, "Hash({hash:#x})"),
            Self::BlockNumber(number) => write!(f, "BlockNumber({number})"),
            Self::NotIncluded(start) => match start {
                NotIncludedStart::Finalized => write!(f, "NotIncluded(Finalized)"),
                NotIncludedStart::BlockDepth(depth) => {
                    write!(f, "NotIncluded(BlockDepth({depth}))")
                }
            },
        }
    }
}

impl From<B256> for L1MessageStart {
    fn from(value: B256) -> Self {
        Self::Hash(value)
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

impl<T> DatabaseReadOperations for T where T: ReadConnectionProvider + ?Sized + Sync {}

impl<T> DatabaseWriteOperations for T where T: WriteConnectionProvider + ?Sized + Sync {}
