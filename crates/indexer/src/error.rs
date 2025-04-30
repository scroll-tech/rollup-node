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
}
