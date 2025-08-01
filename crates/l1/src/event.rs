use scroll_primitives::{BatchInput, L1Message};
use std::sync::Arc;

/// An observation from the L1.
#[derive(Debug)]
pub enum L1Event {
    /// A new batch input has been committed to L1 with the given [`BatchInput`].
    CommitBatch(Arc<BatchInput>),
    /// A new [`L1Message`] has been added to the L1 message queue.
    L1Message(Arc<L1Message>),
    /// A new block has been added to the L1.
    NewBlock(u64),
    /// A reorg has occurred at the given block number.
    Reorg(u64),
    /// The L1 has been finalized at the given block number.
    Finalized(u64),
}
