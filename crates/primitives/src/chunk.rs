use super::BlockContext;
use alloy_primitives::B256;

/// A [`Chunk`] is a series of block commitments that are settled to L1.
///
/// A [`Chunk`] is series of blocks that are proven by a single prover. A collection of [`Chunk`]s
/// are grouped into a [`super::Batch`].
#[derive(Debug)]
pub struct Chunk {
    /// A collection of block commitments.
    pub blocks: Vec<BlockContext>,
    /// The hash of the L1 message queue before the chunk.
    pub prev_l1_message_queue_hash: B256,
    /// The hash of the L1 message queue after the chunk.
    pub post_l1_message_queue_hash: B256,
}
