//! The [`Service`] implementation for the database and the retry mechanism.

use crate::{Database, DatabaseError, DatabaseTransactionProvider};
use std::{fmt::Debug, sync::Arc};

use tower::Service;

mod query;
pub use query::{BoxedFuture, DatabaseQuery, ReadQuery, WriteQuery};

mod retry;
pub use retry::{CanRetry, Retry};

impl<T, Err> Service<DatabaseQuery<T, Err>> for Arc<Database>
where
    T: Send + 'static,
    Err: From<DatabaseError> + Send + 'static,
{
    type Response = T;
    type Error = Err;
    type Future = BoxedFuture<Self::Response, Self::Error>;

    fn poll_ready(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: DatabaseQuery<T, Err>) -> Self::Future {
        let db = self.clone();
        match req {
            DatabaseQuery::Read(f) => Box::pin(async move {
                let tx = Arc::new(db.tx().await?);
                f(tx).await
            }),
            DatabaseQuery::Write(f) => Box::pin(async move {
                let tx = Arc::new(db.tx_mut().await?);
                let res = f(tx.clone()).await;

                // The `WriteQueryFactory` cannot clone the atomic reference to the transaction, or
                // the below will fail, and we won't be able to commit/rollback the transaction.
                let tx = Arc::try_unwrap(tx);

                if res.is_ok() {
                    tx.map_err(|_| DatabaseError::CommitFailed)?.commit().await?;
                } else {
                    tx.map_err(|_| DatabaseError::RollbackFailed)?.rollback().await?;
                }
                res
            }),
        }
    }
}

/// An implementor of the trait can make queries to the database. This trait is used in order to
/// move the `T` generic out from the [`Service<DatabaseQuery<T, Err>>`] trait and into the method
/// call itself.
#[async_trait::async_trait]
pub trait DatabaseService: Clone + Send + Sync + 'static {
    /// Call the database.
    async fn call<T: Send + 'static, Err: From<DatabaseError> + CanRetry + Debug + Send + 'static>(
        &mut self,
        req: DatabaseQuery<T, Err>,
    ) -> Result<T, Err>;
}

#[async_trait::async_trait]
impl DatabaseService for Arc<Database> {
    async fn call<
        T: Send + 'static,
        Err: From<DatabaseError> + CanRetry + Debug + Send + 'static,
    >(
        &mut self,
        req: DatabaseQuery<T, Err>,
    ) -> Result<T, Err> {
        Service::call(self, req).await
    }
}
