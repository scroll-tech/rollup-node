use alloy_eips::BlockId;
use alloy_primitives::B256;

/// The error type for database operations.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    /// A database error occurred.
    #[error("database error: {0}")]
    DatabaseError(#[from] sea_orm::DbErr),
    /// A batch was not found in the database.
    #[error("batch with hash [{0}] not found in database")]
    BatchNotFound(B256),
    /// The block was not found in database.
    #[error("no block for id {0}")]
    BlockNotFound(BlockId),
    /// The L1 message was not found in database.
    #[error("L1 message at index [{0}] not found in database")]
    L1MessageNotFound(u64),
    /// A generic error occurred.
    #[error("parse signature error: {0}")]
    ParseSignatureError(String),
}
