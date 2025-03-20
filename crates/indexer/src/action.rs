use super::{IndexerError, IndexerEvent};
use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// A future that resolves to a `Result<IndexerEvent, IndexerError>`.
type PendingIndexerFuture =
    Pin<Box<dyn Future<Output = Result<IndexerEvent, IndexerError>> + Send>>;

/// A type that represents a future that is being executed by the indexer.
pub(super) enum IndexerFuture {
    HandleReorg(PendingIndexerFuture),
    HandleBatchCommit(PendingIndexerFuture),
    HandleBatchFinalization(PendingIndexerFuture),
    HandleL1Message(PendingIndexerFuture),
}

impl IndexerFuture {
    /// Polls the future to completion.
    pub(super) fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<IndexerEvent, IndexerError>> {
        match self {
            Self::HandleReorg(fut) |
            Self::HandleBatchCommit(fut) |
            Self::HandleBatchFinalization(fut) |
            Self::HandleL1Message(fut) => fut.as_mut().poll(cx),
        }
    }
}

// We implement the Debug trait for IndexerFuture to provide a human-readable representation of the
// enum variants.
impl fmt::Debug for IndexerFuture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandleReorg(_) => write!(f, "HandleReorg"),
            Self::HandleBatchCommit(_) => write!(f, "HandleBatchCommit"),
            Self::HandleBatchFinalization(_) => write!(f, "HandleBatchFinalization"),
            Self::HandleL1Message(_) => write!(f, "HandleL1Message"),
        }
    }
}
