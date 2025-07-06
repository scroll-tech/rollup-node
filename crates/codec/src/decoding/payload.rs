//! Commit payload.

use crate::L2Block;
use alloy_primitives::B256;
use std::vec::Vec;

/// The payload data on the L1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadData {
    /// The L2 blocks from the commit payload.
    pub blocks: Vec<L2Block>,
    /// Contains information about the current state of the L1 message queue.
    pub l1_message_queue_info: L1MessageQueueInfo,
    /// Contains the skipped L1 message bitmap if present.
    pub skipped_l1_message_bitmap: Option<Vec<u8>>,
}

/// Information about the state of the L1 message queue.
#[derive(Debug, Clone, PartialEq, Eq, derive_more::From)]
pub enum L1MessageQueueInfo {
    /// The initial index of the l1 message queue, before commitment of the payload.
    InitialQueueIndex(u64),
    /// The hashed state of the l1 message queue.
    Hashed {
        /// The previous l1 message queue hash.
        prev_l1_message_queue_hash: B256,
        /// The post l1 message queue hash.
        post_l1_message_queue_hash: B256,
    },
}

impl PayloadData {
    /// Returns the list [`L2Block`] committed.
    pub fn l2_blocks(&self) -> &Vec<L2Block> {
        &self.blocks
    }

    /// Returns the list [`L2Block`] committed.
    pub fn into_l2_blocks(self) -> Vec<L2Block> {
        self.blocks
    }

    /// Returns the l1 message queue index of the first message in the batch.
    pub fn queue_index_start(&self) -> Option<u64> {
        match self.l1_message_queue_info {
            L1MessageQueueInfo::InitialQueueIndex(index) => Some(index),
            L1MessageQueueInfo::Hashed { .. } => None,
        }
    }

    /// Returns the l1 message queue hash before the commitment of the batch.
    pub fn prev_l1_message_queue_hash(&self) -> Option<&B256> {
        match self.l1_message_queue_info {
            L1MessageQueueInfo::InitialQueueIndex(_) => None,
            L1MessageQueueInfo::Hashed { ref prev_l1_message_queue_hash, .. } => {
                Some(prev_l1_message_queue_hash)
            }
        }
    }

    /// Returns the l1 message queue hash after the commitment of the batch.
    pub fn post_l1_message_queue_hash(&self) -> Option<&B256> {
        match self.l1_message_queue_info {
            L1MessageQueueInfo::InitialQueueIndex(_) => None,
            L1MessageQueueInfo::Hashed { ref post_l1_message_queue_hash, .. } => {
                Some(post_l1_message_queue_hash)
            }
        }
    }
}
