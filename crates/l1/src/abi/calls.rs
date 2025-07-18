use std::vec::Vec;

use alloy_sol_types::{sol, SolCall};

sol! {
    #[cfg_attr(feature = "test-utils", derive(arbitrary::Arbitrary))]
    #[derive(Debug)]
    function commitBatch(
        uint8 version,
        bytes calldata parent_batch_header,
        bytes[] memory chunks,
        bytes calldata skipped_l1_message_bitmap
    ) external;

    #[cfg_attr(feature = "test-utils", derive(arbitrary::Arbitrary))]
    #[derive(Debug)]
    function commitBatchWithBlobProof(
        uint8 version,
        bytes calldata parent_batch_header,
        bytes[] memory chunks,
        bytes calldata skipped_l1_message_bitmap,
        bytes calldata blob_data_proof
    ) external;

    #[cfg_attr(feature = "test-utils", derive(arbitrary::Arbitrary))]
    #[derive(Debug)]
    function commitBatches(
        uint8 version,
        bytes32 parent_batch_hash,
        bytes32 last_batch_hash
    ) external;
}

/// A call to commit a batch on the L1 Scroll Rollup contract.
#[derive(Debug, derive_more::From)]
pub enum CommitBatchCall {
    /// A plain call to commit the batch.
    CommitBatch(commitBatchCall),
    /// A call to commit the batch with a blob proof.
    CommitBatchWithBlobProof(commitBatchWithBlobProofCall),
    /// A call to commit the multiple batches.
    CommitBatches(commitBatchesCall),
}

impl CommitBatchCall {
    /// Tries to decode the calldata into a [`CommitBatchCall`].
    pub fn try_decode(calldata: &[u8]) -> Option<Self> {
        match calldata.get(0..4).map(|sel| sel.try_into().expect("correct slice length")) {
            Some(commitBatchCall::SELECTOR) => {
                commitBatchCall::abi_decode(calldata).map(Into::into).ok()
            }
            Some(commitBatchWithBlobProofCall::SELECTOR) => {
                commitBatchWithBlobProofCall::abi_decode(calldata).map(Into::into).ok()
            }
            Some(commitBatchesCall::SELECTOR) => {
                commitBatchesCall::abi_decode(calldata).map(Into::into).ok()
            }
            Some(_) | None => None,
        }
    }

    /// Returns the version for the commit call.
    pub const fn version(&self) -> u8 {
        match self {
            Self::CommitBatch(b) => b.version,
            Self::CommitBatchWithBlobProof(b) => b.version,
            Self::CommitBatches(b) => b.version,
        }
    }

    /// Returns the parent batch header for the commit call.
    pub fn parent_batch_header(&self) -> Option<Vec<u8>> {
        match &self {
            Self::CommitBatch(b) => Some(b.parent_batch_header.to_vec()),
            Self::CommitBatchWithBlobProof(b) => Some(b.parent_batch_header.to_vec()),
            Self::CommitBatches(_) => None,
        }
    }

    /// Returns the chunks for the commit call if any, returns None otherwise.
    pub fn chunks(&self) -> Option<Vec<&[u8]>> {
        let chunks = match self {
            Self::CommitBatch(b) => &b.chunks,
            Self::CommitBatchWithBlobProof(b) => &b.chunks,
            Self::CommitBatches(_) => return None,
        };
        Some(chunks.iter().map(|c| c.as_ref()).collect())
    }

    /// Returns the skipped L1 message bitmap for the commit call if any, returns None otherwise.
    pub fn skipped_l1_message_bitmap(&self) -> Option<Vec<u8>> {
        let bitmap = match self {
            Self::CommitBatch(b) => &b.skipped_l1_message_bitmap,
            Self::CommitBatchWithBlobProof(b) => &b.skipped_l1_message_bitmap,
            Self::CommitBatches(_) => return None,
        };
        let mut bitmap = bitmap.to_vec();
        bitmap.reverse();
        Some(bitmap)
    }
}
