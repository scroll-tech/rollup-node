use alloy_primitives::B256;
use scroll_db::DatabaseError;

/// A type that represents an error that occurred during indexing.
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    /// An error occurred while interacting with the database.
    #[error("indexing failed due to database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    /// An error occurred while trying to fetch the L2 block from the database.
    #[error("L2 block not found - block number: {0}")]
    L2BlockNotFound(u64),
    /// A fork was received from the peer that is associated with a reorg of the safe chain.
    #[error("L2 safe block reorg detected")]
    L2SafeBlockReorgDetected,
    /// A block contains invalid L1 messages.
    #[error("Block contains invalid L1 message. Expected: {expected:?}, Actual: {actual:?}")]
    L1MessageMismatch {
        /// The expected L1 messages hash.
        expected: B256,
        /// The actual L1 messages hash.
        actual: B256,
    },
    /// An inconsistency was detected when trying to consolidate the chain.
    #[error("Chain inconsistency detected")]
    ChainInconsistency,
}
