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
    HandleUnwind(PendingIndexerFuture),
    HandleFinalized(PendingIndexerFuture),
    HandleBatchCommit(PendingIndexerFuture),
    HandleBatchFinalization(PendingIndexerFuture),
    HandleUnwindBatch(PendingIndexerFuture),
    HandleL1Message(PendingIndexerFuture),
    HandleDerivedBlock(PendingIndexerFuture),
}

impl IndexerFuture {
    /// Polls the future to completion.
    pub(super) fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<IndexerEvent, IndexerError>> {
        match self {
            Self::HandleUnwind(fut) |
            Self::HandleFinalized(fut) |
            Self::HandleBatchCommit(fut) |
            Self::HandleBatchFinalization(fut) |
            Self::HandleUnwindBatch(fut) |
            Self::HandleL1Message(fut) |
            Self::HandleDerivedBlock(fut) => fut.as_mut().poll(cx),
        }
    }
}

// We implement the Debug trait for IndexerFuture to provide a human-readable representation of the
// enum variants.
impl fmt::Debug for IndexerFuture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandleUnwind(_) => write!(f, "HandleUnwind"),
            Self::HandleFinalized(_) => write!(f, "HandleFinalized"),
            Self::HandleBatchCommit(_) => write!(f, "HandleBatchCommit"),
            Self::HandleBatchFinalization(_) => write!(f, "HandleBatchFinalization"),
            Self::HandleUnwindBatch(_) => write!(f, "HandleUnwindBatch"),
            Self::HandleL1Message(_) => write!(f, "HandleL1Message"),
            Self::HandleDerivedBlock(_) => write!(f, "HandleDerivedBlock"),
        }
    }
}
