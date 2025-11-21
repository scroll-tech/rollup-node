use metrics::Histogram;
use metrics_derive::Metrics;
use strum::EnumIter;

/// The metrics for the [`super::db::DatabaseInner`].
#[derive(Metrics, Clone)]
#[metrics(scope = "database")]
pub(crate) struct DatabaseMetrics {
    /// Time (ms) to acquire a DB read lock/connection.
    #[metric(describe = "Time to acquire a database read lock (ms)")]
    pub read_lock_acquire_duration: Histogram,
    /// Time (ms) to acquire a DB write lock/connection.
    #[metric(describe = "Time to acquire a database write lock (ms)")]
    pub write_lock_acquire_duration: Histogram,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, EnumIter)]
pub(crate) enum DatabaseOperation {
    // Write operations
    InsertL1BlockInfo,
    RemoveL1BlockInfoLeq,
    DeleteBatchFinalizationGtBlockNumber,
    SetBatchRevertBlockNumberForBatchRange,
    DeleteBatchRevertGtBlockNumber,
    FinalizeConsolidatedBatches,
    ChangeBatchProcessingToCommittedStatus,
    RemoveL1BlockInfoGt,
    UpdateBatchStatus,
    InsertBatch,
    FinalizeBatchesUpToIndex,
    SetLatestL1BlockNumber,
    SetFinalizedL1BlockNumber,
    SetProcessedL1BlockNumber,
    SetL2HeadBlockNumber,
    FetchAndUpdateUnprocessedFinalizedBatches,
    FetchAndUpdateUnprocessedCommittedBatches,
    DeleteBatchesGtBlockNumber,
    DeleteBatchesGtBatchIndex,
    InsertL1Message,
    UpdateSkippedL1Messages,
    DeleteL1MessagesGt,
    PrepareOnStartup,
    DeleteL2BlocksGtBlockNumber,
    DeleteL2BlocksGtBatchIndex,
    InsertBlocks,
    InsertBlock,
    InsertGenesisBlock,
    UpdateL1MessagesFromL2Blocks,
    UpdateL1MessagesWithL2Block,
    PurgeL1MessageToL2BlockMappings,
    InsertBatchConsolidationOutcome,
    Unwind,
    InsertSignatures,
    InsertSignature,
    // Read operations
    GetBatchByIndex,
    GetBatchByHash,
    GetBatchStatusByHash,
    GetLatestIndexedEventL1BlockNumber,
    GetL1BlockInfo,
    GetLatestL1BlockNumber,
    GetFinalizedL1BlockNumber,
    GetProcessedL1BlockNumber,
    GetL2HeadBlockNumber,
    GetLastBatchCommitL1Block,
    GetLastL1MessageL1Block,
    GetNL1Messages,
    GetNL2BlockDataHint,
    GetL2BlockAndBatchInfoByHash,
    GetL2BlockInfoByNumber,
    GetLatestSafeL2Info,
    GetHighestBlockForBatchHash,
    GetHighestBlockForBatchIndex,
    GetSignature,
}

impl DatabaseOperation {
    /// Returns the str representation of the [`DatabaseOperation`].
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::InsertL1BlockInfo => "insert_l1_block_info",
            Self::RemoveL1BlockInfoLeq => "remove_l1_block_info_leq",
            Self::DeleteBatchFinalizationGtBlockNumber => {
                "delete_batch_finalization_gt_block_number"
            }
            Self::SetBatchRevertBlockNumberForBatchRange => {
                "set_batch_revert_block_number_for_batch_range"
            }
            Self::DeleteBatchRevertGtBlockNumber => "delete_batch_revert_gt_block_number",
            Self::FinalizeConsolidatedBatches => "finalize_consolidated_batches",
            Self::ChangeBatchProcessingToCommittedStatus => {
                "change_batch_processing_to_committed_status"
            }
            Self::UpdateBatchStatus => "update_batch_status",
            Self::RemoveL1BlockInfoGt => "remove_l1_block_info_gt",
            Self::InsertBatch => "insert_batch",
            Self::FinalizeBatchesUpToIndex => "finalize_batches_up_to_index",
            Self::SetLatestL1BlockNumber => "set_latest_l1_block_number",
            Self::SetFinalizedL1BlockNumber => "set_finalized_l1_block_number",
            Self::SetProcessedL1BlockNumber => "set_processed_l1_block_number",
            Self::SetL2HeadBlockNumber => "set_l2_head_block_number",
            Self::FetchAndUpdateUnprocessedFinalizedBatches => {
                "fetch_and_update_unprocessed_finalized_batches"
            }
            Self::FetchAndUpdateUnprocessedCommittedBatches => {
                "fetch_and_update_unprocessed_committed_batches"
            }
            Self::DeleteBatchesGtBlockNumber => "delete_batches_gt_block_number",
            Self::DeleteBatchesGtBatchIndex => "delete_batches_gt_batch_index",
            Self::InsertL1Message => "insert_l1_message",
            Self::UpdateSkippedL1Messages => "update_skipped_l1_messages",
            Self::DeleteL1MessagesGt => "delete_l1_messages_gt",
            Self::PrepareOnStartup => "prepare_on_startup",
            Self::DeleteL2BlocksGtBlockNumber => "delete_l2_blocks_gt_block_number",
            Self::DeleteL2BlocksGtBatchIndex => "delete_l2_blocks_gt_batch_index",
            Self::InsertBlocks => "insert_blocks",
            Self::InsertBlock => "insert_block",
            Self::InsertGenesisBlock => "insert_genesis_block",
            Self::UpdateL1MessagesFromL2Blocks => "update_l1_messages_from_l2_blocks",
            Self::UpdateL1MessagesWithL2Block => "update_l1_messages_with_l2_block",
            Self::PurgeL1MessageToL2BlockMappings => "purge_l1_message_to_l2_block_mappings",
            Self::InsertBatchConsolidationOutcome => "insert_batch_consolidation_outcome",
            Self::Unwind => "unwind",
            Self::InsertSignatures => "insert_signatures",
            Self::InsertSignature => "insert_signature",
            Self::GetBatchByIndex => "get_batch_by_index",
            Self::GetBatchByHash => "get_batch_by_hash",
            Self::GetBatchStatusByHash => "get_batch_status_by_hash",
            Self::GetLatestIndexedEventL1BlockNumber => "get_latest_indexed_event_l1_block_number",
            Self::GetL1BlockInfo => "get_l1_block_info",
            Self::GetLatestL1BlockNumber => "get_latest_l1_block_number",
            Self::GetFinalizedL1BlockNumber => "get_finalized_l1_block_number",
            Self::GetProcessedL1BlockNumber => "get_processed_l1_block_number",
            Self::GetL2HeadBlockNumber => "get_l2_head_block_number",
            Self::GetLastBatchCommitL1Block => "get_last_batch_commit_l1_block",
            Self::GetLastL1MessageL1Block => "get_last_l1_message_l1_block",
            Self::GetNL1Messages => "get_n_l1_messages",
            Self::GetNL2BlockDataHint => "get_n_l2_block_data_hint",
            Self::GetL2BlockAndBatchInfoByHash => "get_l2_block_and_batch_info_by_hash",
            Self::GetL2BlockInfoByNumber => "get_l2_block_info_by_number",
            Self::GetLatestSafeL2Info => "get_latest_safe_l2_info",
            Self::GetHighestBlockForBatchHash => "get_highest_block_for_batch_hash",
            Self::GetHighestBlockForBatchIndex => "get_highest_block_for_batch_index",
            Self::GetSignature => "get_signature",
        }
    }
}

/// The metrics for the [`super::Database`].
#[derive(Metrics, Clone)]
#[metrics(scope = "database")]
pub(crate) struct DatabaseOperationMetrics {
    /// The duration of the task for the chain orchestrator.
    pub task_duration: Histogram,
}
