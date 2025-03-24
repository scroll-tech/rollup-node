//! Commit payload.

use crate::L2Block;
use alloy_primitives::B256;
use std::vec::Vec;

/// The payload data committed on the L1.
#[derive(Debug, Clone, PartialEq, Eq, derive_more::From)]
pub enum CommitPayload {
    /// The base payload which only contains a vector of [`L2Block`].
    Base(Vec<L2Block>),
    /// The commit base commit payload with L1 messages hashes.
    WithL1MessagesHashes {
        /// The L2 blocks from the commit payload.
        blocks: Vec<L2Block>,
        /// The previous l1 message queue hash.
        prev_l1_message_queue_hash: B256,
        /// The post l1 message queue hash.
        post_l1_message_queue_hash: B256,
    },
}

impl CommitPayload {
    /// Returns the list [`L2Block`] committed.
    pub fn l2_blocks(&self) -> &Vec<L2Block> {
        match self {
            CommitPayload::Base(blocks) => blocks,
            CommitPayload::WithL1MessagesHashes { blocks, .. } => blocks,
        }
    }

    /// Returns the l1 message queue hash before the commitment of the batch.
    pub fn prev_l1_message_queue_hash(&self) -> Option<&B256> {
        match self {
            CommitPayload::Base(_) => None,
            CommitPayload::WithL1MessagesHashes { prev_l1_message_queue_hash, .. } => {
                Some(prev_l1_message_queue_hash)
            }
        }
    }

    /// Returns the l1 message queue hash after the commitment of the batch.
    pub fn post_l1_message_queue_hash(&self) -> Option<&B256> {
        match self {
            CommitPayload::Base(_) => None,
            CommitPayload::WithL1MessagesHashes { post_l1_message_queue_hash, .. } => {
                Some(post_l1_message_queue_hash)
            }
        }
    }
}
