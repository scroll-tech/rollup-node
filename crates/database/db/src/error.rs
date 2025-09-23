use super::L1MessageStart;
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
    /// The L1 message was not found in database.
    #[error("L1 message at index [{0}] not found in database")]
    L1MessageNotFound(L1MessageStart),
}
