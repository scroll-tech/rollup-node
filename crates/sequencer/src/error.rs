use scroll_db::DatabaseError;
use scroll_engine::EngineDriverError;

// TODO: make the error types more fine grained.

/// An error type for the sequencer.
#[derive(Debug, thiserror::Error)]
pub enum SequencerError {
    /// The sequencer encountered an error when interacting with the database.
    #[error("Encountered an error interacting with the database: {0}")]
    DatabaseError(#[from] DatabaseError),
    /// The sequencer encountered an error when interacting with the engine driver.
    #[error("Encountered an error interacting with the EngineDriver {0}")]
    EngineDriverError(#[from] EngineDriverError),
}
