use scroll_primitives::{BatchInput, L1Message};
use std::sync::Arc;

/// An observation from the L1.
#[derive(Debug)]
pub enum L1Event {
    CommitBatch(Arc<BatchInput>),
    L1Message(Arc<L1Message>),
    Reorg(u64),
    Finalized(u64),
}
