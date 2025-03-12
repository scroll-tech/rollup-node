use scroll_primitives::L1Message;
use std::sync::Arc;

/// An observation from the L1.
#[derive(Debug)]
pub enum L1Event {
    /// A block containing a pipeline event has been added to the L1.
    PipelineEvent(u64),
    /// A new L1 message has been added to the L1.
    L1Message(Arc<L1Message>),
    /// A new block has been added to the L1.
    NewBlock(u64),
    /// A reorg has occurred at the given block number.
    Reorg(u64),
    /// The L1 has been finalized at the given block number.
    Finalized(u64),
}
