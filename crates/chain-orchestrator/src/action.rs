use super::{ChainOrchestratorError, ChainOrchestratorEvent};
use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// A future that resolves to a `Result<ChainOrchestratorEvent, ChainOrchestratorError>`.
pub(super) type PendingChainOrchestratorFuture = Pin<
    Box<dyn Future<Output = Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError>> + Send>,
>;

/// A type that represents a future that is being executed by the chain orchestrator.
pub(super) enum ChainOrchestratorFuture {
    HandleReorg(PendingChainOrchestratorFuture),
    HandleFinalized(PendingChainOrchestratorFuture),
    HandleBatchCommit(PendingChainOrchestratorFuture),
    HandleBatchFinalization(PendingChainOrchestratorFuture),
    HandleL1Message(PendingChainOrchestratorFuture),
    HandleDerivedBlock(PendingChainOrchestratorFuture),
    HandleL2Block(PendingChainOrchestratorFuture),
}

impl ChainOrchestratorFuture {
    /// Polls the future to completion.
    pub(super) fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError>> {
        match self {
            Self::HandleReorg(fut) |
            Self::HandleFinalized(fut) |
            Self::HandleBatchCommit(fut) |
            Self::HandleBatchFinalization(fut) |
            Self::HandleL1Message(fut) |
            Self::HandleDerivedBlock(fut) |
            Self::HandleL2Block(fut) => fut.as_mut().poll(cx),
        }
    }
}

// We implement the Debug trait for ChainOrchestratorFuture to provide a human-readable
// representation of the enum variants.
impl fmt::Debug for ChainOrchestratorFuture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandleReorg(_) => write!(f, "HandleReorg"),
            Self::HandleFinalized(_) => write!(f, "HandleFinalized"),
            Self::HandleBatchCommit(_) => write!(f, "HandleBatchCommit"),
            Self::HandleBatchFinalization(_) => write!(f, "HandleBatchFinalization"),
            Self::HandleL1Message(_) => write!(f, "HandleL1Message"),
            Self::HandleDerivedBlock(_) => write!(f, "HandleDerivedBlock"),
            Self::HandleL2Block(_) => write!(f, "HandleL2Block"),
        }
    }
}
