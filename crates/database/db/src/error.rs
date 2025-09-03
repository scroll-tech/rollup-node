use super::L1MessageStart;
use alloy_eips::BlockId;
use alloy_primitives::B256;
use sea_orm::sqlx::Error as SqlxError;

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
    /// A generic error occurred.
    #[error("parse signature error: {0}")]
    ParseSignatureError(String),
    /// The L1 message was not found in database.
    #[error("L1 message at index [{0}] not found in database")]
    L1MessageNotFound(L1MessageStart),
    /// An error occurred at the sqlx level.
    #[error("A sqlx error occurred: {0}")]
    SqlxError(#[from] SqlxError),
}
