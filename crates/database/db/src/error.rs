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
}
