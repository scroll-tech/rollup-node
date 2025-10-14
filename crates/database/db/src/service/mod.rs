//! The [`Service`] implementation for the database and the retry mechanism.

use crate::{
    db::DatabaseInner, service::query::DatabaseQuery, DatabaseError, DatabaseTransactionProvider,
};
use std::{fmt::Debug, sync::Arc};

pub(crate) mod query;

pub(crate) mod retry;
pub use retry::CanRetry;

/// Error type for database service operations that can be converted from [`DatabaseError`],
/// supports retry logic, and is thread-safe for async contexts.
pub trait DatabaseServiceError: From<DatabaseError> + CanRetry + Debug + Send + 'static {}
impl<T> DatabaseServiceError for T where T: From<DatabaseError> + CanRetry + Debug + Send + 'static {}

/// An implementer of the trait can make queries to the database. This trait is used in order to
/// move the `T` generic out from the [`Service<DatabaseQuery<T, Err>>`] trait and into the method
/// itself.
#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Arc)]
pub(crate) trait DatabaseService: Clone + Send + Sync + 'static {
    /// Call the database.
    async fn call<T: Send + 'static, Err: DatabaseServiceError>(
        &self,
        req: DatabaseQuery<T, Err>,
    ) -> Result<T, Err>;
}

#[async_trait::async_trait]
impl DatabaseService for Arc<DatabaseInner> {
    async fn call<T: Send + 'static, Err: DatabaseServiceError>(
        &self,
        req: DatabaseQuery<T, Err>,
    ) -> Result<T, Err> {
        let db = self.clone();
        match req {
            DatabaseQuery::Read(f) => {
                let tx = Arc::new(db.tx().await?);
                f(tx).await
            }
            DatabaseQuery::Write(f) => {
                let tx = Arc::new(db.tx_mut().await?);
                let res = f(tx.clone()).await;

                // The `WriteQuery` cannot clone the atomic reference to the transaction, or the
                // below will fail, and we won't be able to commit/rollback the transaction.
                let tx = Arc::try_unwrap(tx);

                if res.is_ok() {
                    tx.map_err(|_| DatabaseError::CommitFailed)?.commit().await?;
                } else {
                    tx.map_err(|_| DatabaseError::RollbackFailed)?.rollback().await?;
                }
                res
            }
        }
    }
}
