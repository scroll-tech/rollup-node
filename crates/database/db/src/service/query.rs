use crate::{TXMut, TX};

use std::{
    fmt::{Debug, Formatter},
    future::Future,
    pin::Pin,
    sync::Arc,
};

/// A boxed future which returns a database query result.
pub(crate) type BoxedFuture<T, Err> = Pin<Box<dyn Future<Output = Result<T, Err>> + Send>>;

/// A read query that uses a [`TX`] for the call.
pub(crate) type ReadQuery<T, Err> = Arc<dyn Fn(Arc<TX>) -> BoxedFuture<T, Err> + Send + Sync>;

/// A write query that uses a [`TXMut`] for the call.
pub(crate) type WriteQuery<T, Err> = Arc<dyn Fn(Arc<TXMut>) -> BoxedFuture<T, Err> + Send + Sync>;

/// A query to the database.
pub(crate) enum DatabaseQuery<T, Err> {
    /// A read query to the database.
    Read(ReadQuery<T, Err>),
    /// A write query to the database.
    Write(WriteQuery<T, Err>),
}

impl<T, Err> Clone for DatabaseQuery<T, Err> {
    fn clone(&self) -> Self {
        match self {
            Self::Read(f) => Self::Read(f.clone()),
            Self::Write(f) => Self::Write(f.clone()),
        }
    }
}

impl<T: Debug, Err: Debug> Debug for DatabaseQuery<T, Err> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(_) => write!(f, "DatabaseQuery::Read"),
            Self::Write(_) => write!(f, "DatabaseQuery::Write"),
        }
    }
}

impl<T, Err> DatabaseQuery<T, Err>
where
    T: Send + 'static,
    Err: Send + 'static,
{
    /// Create a new read database query.
    pub(crate) fn read<F, Fut>(f: F) -> Self
    where
        F: Fn(Arc<TX>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Err>> + Send + 'static,
    {
        Self::Read(Arc::new(move |tx| Box::pin(f(tx))))
    }

    /// Create a new write database query.
    pub(crate) fn write<F, Fut>(f: F) -> Self
    where
        F: Fn(Arc<TXMut>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Err>> + Send + 'static,
    {
        Self::Write(Arc::new(move |tx| Box::pin(f(tx))))
    }
}
