use super::L1MessageKey;
use sea_orm::sqlx::Error as SqlxError;

/// The error type for database operations.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    /// A database error occurred.
    #[error("database error: {0}")]
    DatabaseError(#[from] sea_orm::DbErr),
    /// An error occurred at the sqlx level.
    #[error("A sqlx error occurred: {0}")]
    SqlxError(#[from] SqlxError),
    /// A generic error occurred.
    #[error("parse signature error: {0}")]
    ParseSignatureError(String),
    /// Failed to serde the metadata value.
    #[error("failed to serde metadata value: {0}")]
    MetadataSerdeError(#[from] serde_json::Error),
    /// The L1 message was not found in database.
    #[error("L1 message at key [{0}] not found in database")]
    L1MessageNotFound(L1MessageKey),
    /// The finalized L1 block was not found in database.
    #[error("Finalized L1 block not found in database")]
    FinalizedL1BlockNotFound,
    /// The latest L1 block was not found in database.
    #[error("Latest L1 block not found in database")]
    LatestL1BlockNotFound,
}
