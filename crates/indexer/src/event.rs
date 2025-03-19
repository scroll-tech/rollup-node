use alloy_primitives::B256;

/// An event emitted by the indexer.
#[derive(Debug, Clone, Copy)]
pub enum IndexerEvent {
    /// A `BatchCommit` event has been indexed returning the batch index.
    BatchCommitIndexed(u64),
    /// A `BatchFinalization` event has been indexed returning the batch index.
    BatchFinalizationIndexed(B256),
    /// A `L1Message` event has been indexed returning the message queue index.
    L1MessageIndexed(u64),
    /// A `Reorg` event has been indexed returning the reorg block number.
    ReorgIndexed(u64),
}
