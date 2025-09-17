use rollup_node_providers::L1ProviderError;
use scroll_db::DatabaseError;

// TODO: make the error types more fine grained.

/// An error type for the sequencer.
#[derive(Debug, thiserror::Error)]
pub enum SequencerError {
    /// The sequencer encountered an error when interacting with the database.
    #[error("Encountered an error interacting with the database: {0}")]
    DatabaseError(#[from] DatabaseError),
    /// The sequencer encountered an error when interacting with the L1 message provider.
    #[error("Encountered an error interacting with the L1 message provider: {0}")]
    L1MessageProviderError(#[from] L1ProviderError),
    /// The received L1 messages are not contiguous.
    #[error("L1 messages are not contiguous: got {got}, expected {expected}")]
    NonContiguousL1Messages {
        /// The L1 message queue index received.
        got: u64,
        /// The expected L1 message queue index.
        expected: u64,
    },
}
