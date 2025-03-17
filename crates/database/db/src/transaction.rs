use super::{DatabaseConnectionProvider, DatabaseError};

/// A type that represents a database transaction.
///
/// This type is used to perform operations on the database within a single atomic transaction.
#[derive(Debug)]
pub struct DatabaseTransaction {
    /// The underlying database transaction.
    tx: sea_orm::DatabaseTransaction,
}

impl DatabaseTransaction {
    /// Creates a new [`DatabaseTransaction`] instance associated with the provided database
    /// transaction.
    pub const fn new(tx: sea_orm::DatabaseTransaction) -> Self {
        Self { tx }
    }
}

impl DatabaseTransaction {
    /// Commits the transaction.
    pub async fn commit(self) -> Result<(), DatabaseError> {
        self.tx.commit().await?;
        Ok(())
    }

    /// Rolls back the transaction.
    pub async fn rollback(self) -> Result<(), DatabaseError> {
        self.tx.rollback().await?;
        Ok(())
    }
}

impl DatabaseConnectionProvider for DatabaseTransaction {
    fn get_connection(&self) -> &(impl sea_orm::ConnectionTrait + sea_orm::StreamTrait) {
        &self.tx
    }
}
