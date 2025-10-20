use crate::DatabaseConnectionProvider;

use super::{DatabaseError, ReadConnectionProvider, WriteConnectionProvider};
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit};

/// A type that represents a read-only database transaction.
///
/// This type is used to perform read operations on the database.
#[derive(Debug)]
pub struct TX {
    /// The underlying database transaction.
    tx: sea_orm::DatabaseTransaction,
    /// A permit for the read transaction semaphore.
    _permit: OwnedSemaphorePermit,
}

impl TX {
    /// Creates a new [`TX`] instance associated with the provided [`sea_orm::DatabaseTransaction`].
    pub const fn new(tx: sea_orm::DatabaseTransaction, permit: OwnedSemaphorePermit) -> Self {
        Self { tx, _permit: permit }
    }
}

impl DatabaseConnectionProvider for TX {
    type Connection = sea_orm::DatabaseTransaction;

    fn get_connection(&self) -> &Self::Connection {
        &self.tx
    }
}

impl ReadConnectionProvider for TX {}

/// A type that represents a mutable database transaction.
///
/// This type is used to perform atomic read and write operations on the database.
#[derive(Debug)]
pub struct TXMut {
    /// The underlying database transaction.
    tx: sea_orm::DatabaseTransaction,
    /// A guard for the transaction's mutex.
    _guard: OwnedMutexGuard<()>,
}

impl TXMut {
    /// Creates a new [`TXMut`] instance associated with the provided
    /// [`sea_orm::DatabaseTransaction`] and mutex guard.
    pub const fn new(tx: sea_orm::DatabaseTransaction, guard: OwnedMutexGuard<()>) -> Self {
        Self { tx, _guard: guard }
    }
}

impl TXMut {
    /// Commits the transaction.
    pub async fn commit(self) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", "Committing transaction");
        self.tx.commit().await?;
        Ok(())
    }

    /// Rolls back the transaction.
    pub async fn rollback(self) -> Result<(), DatabaseError> {
        tracing::trace!(target: "scroll::db", "Rolling back transaction");
        self.tx.rollback().await?;
        Ok(())
    }
}

impl DatabaseConnectionProvider for TXMut {
    type Connection = sea_orm::DatabaseTransaction;

    fn get_connection(&self) -> &Self::Connection {
        &self.tx
    }
}

impl ReadConnectionProvider for TXMut {}
impl WriteConnectionProvider for TXMut {}

/// A trait for types that can provide database transactions.
#[async_trait::async_trait]
pub trait DatabaseTransactionProvider {
    /// Begins a new read-only transaction.
    async fn tx(&self) -> Result<TX, DatabaseError>;

    /// Begins a new read-write transaction.
    async fn tx_mut(&self) -> Result<TXMut, DatabaseError>;
}
