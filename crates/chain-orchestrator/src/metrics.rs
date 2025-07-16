use metrics::Histogram;
use metrics_derive::Metrics;
use strum::EnumIter;

/// An enum representing the items the indexer can handle.
#[derive(Debug, PartialEq, Eq, Hash, EnumIter)]
pub enum IndexerItem {
    /// Handle a block received from the network.
    NewBlock,
    /// L2 block.
    InsertL2Block,
    /// L1 reorg.
    L1Reorg,
    /// L1 finalization.
    L1Finalization,
    /// L1 message.
    L1Message,
    /// Batch commit.
    BatchCommit,
    /// Batch finalization.
    BatchFinalization,
}

impl IndexerItem {
    /// Returns the str representation of the [`IndexerItem`].
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NewBlock => "new_block",
            Self::InsertL2Block => "l2_block",
            Self::L1Reorg => "l1_reorg",
            Self::L1Finalization => "l1_finalization",
            Self::L1Message => "l1_message",
            Self::BatchCommit => "batch_commit",
            Self::BatchFinalization => "batch_finalization",
        }
    }
}

/// The metrics for the [`super::ChainOrchestrator`].
#[derive(Metrics, Clone)]
#[metrics(scope = "indexer")]
pub struct IndexerMetrics {
    /// The duration of the task for the indexer.
    pub task_duration: Histogram,
}
