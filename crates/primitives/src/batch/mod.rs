use super::{BlockCommitment, Chunk};
use alloy_primitives::B256;

mod input;
pub use input::BatchInput;

/// A batch is the unit of settlement to L1 for the scroll rollup.
///
/// A batch contains a list of chunks, which contain a list of block commitments.
/// A batch additionally contains metadata related to the batch.
#[derive(Debug)]
pub struct Batch {
    /// The index of the batch.
    pub index: u64,
    /// The total number of L1 messages popped before this batch.
    pub total_l1_messages_popped_before: u64,
    /// The hash of the parent batch.
    pub parent_hash: B256,
    /// The chunks in the batch.
    pub chunks: Vec<Chunk>,
    /// The hash of the L1 message queue before the batch.
    pub prev_l1_message_queue_hash: B256,
    /// The hash of the L1 message queue after the batch.
    pub post_l1_message_queue_hash: B256,
    /// The block commitments in the batch.
    pub blocks: Vec<BlockCommitment>,
}

impl Batch {
    /// Creates a new [`Batch`] instance.
    pub const fn new(
        index: u64,
        total_l1_messages_popped_before: u64,
        parent_hash: B256,
        chunks: Vec<Chunk>,
        prev_l1_message_queue_hash: B256,
        post_l1_message_queue_hash: B256,
        blocks: Vec<BlockCommitment>,
    ) -> Self {
        Self {
            index,
            total_l1_messages_popped_before,
            parent_hash,
            chunks,
            prev_l1_message_queue_hash,
            post_l1_message_queue_hash,
            blocks,
        }
    }
}
