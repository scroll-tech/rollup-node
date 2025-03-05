use alloy_primitives::{Bytes, Log};
use alloy_sol_types::{sol, SolCall, SolEvent};
use scroll_alloy_consensus::TxL1Message;

sol! {
    // *********************EVENTS*********************
    event QueueTransaction(
        address indexed sender,
        address indexed target,
        uint256 value,
        uint64 queueIndex,
        uint256 gasLimit,
        bytes data
    );

    event CommitBatch(uint256 indexed batchIndex, bytes32 indexed batchHash);

    // *********************FUNCTION*********************
    function commitBatch(
        uint8 version,
        bytes calldata parentBatchHeader,
        bytes[] memory chunks,
        bytes calldata skippedL1MessageBitmap
    ) external;

    function commitBatchWithBlobProof(
        uint8 version,
        bytes calldata parentBatchHeader,
        bytes[] memory chunks,
        bytes calldata skippedL1MessageBitmap,
        bytes calldata blobDataProof
    ) external;
}

/// A call to commit a batch on the L1 Scroll Rollup contract.
#[derive(derive_more::From)]
pub(super) enum CommitBatchCall {
    /// A plain call to commit the batch.
    CommitBatch(commitBatchCall),
    /// A call to commit the batch with a blob proof.
    CommitBatchWithBlobProof(commitBatchWithBlobProofCall),
}

impl CommitBatchCall {
    pub(crate) fn version(&self) -> u8 {
        match self {
            CommitBatchCall::CommitBatch(b) => b.version,
            CommitBatchCall::CommitBatchWithBlobProof(b) => b.version,
        }
    }
    pub(crate) fn parent_batch_header(&self) -> Vec<u8> {
        let header = match self {
            CommitBatchCall::CommitBatch(b) => &b.parentBatchHeader,
            CommitBatchCall::CommitBatchWithBlobProof(b) => &b.parentBatchHeader,
        };
        header.to_vec()
    }
    pub(crate) fn chunks(&self) -> Vec<Vec<u8>> {
        let chunks = match self {
            CommitBatchCall::CommitBatch(b) => &b.chunks,
            CommitBatchCall::CommitBatchWithBlobProof(b) => &b.chunks,
        };
        chunks.iter().map(|c| c.to_vec()).collect()
    }
    pub(crate) fn skipped_l1_message_bitmap(&self) -> Vec<u8> {
        let bitmap = match self {
            CommitBatchCall::CommitBatch(b) => &b.skippedL1MessageBitmap,
            CommitBatchCall::CommitBatchWithBlobProof(b) => &b.skippedL1MessageBitmap,
        };
        bitmap.to_vec()
    }
}

/// Tries to decode the provided log into the type T.
pub(super) fn try_decode_log<T: SolEvent>(log: &Log) -> Option<Log<T>> {
    T::decode_log(log, true).ok()
}

/// Tries to decode the provided calldata and convert it to a [`CommitBatchCall`].
pub(super) fn try_decode_commit_call(calldata: &Bytes) -> Option<CommitBatchCall> {
    match calldata.get(0..4).map(|sel| sel.try_into().expect("correct slice length")) {
        Some(commitBatchCall::SELECTOR) => {
            commitBatchCall::abi_decode(calldata, true).map(Into::into).ok()
        }
        Some(commitBatchWithBlobProofCall::SELECTOR) => {
            commitBatchWithBlobProofCall::abi_decode(calldata, true).map(Into::into).ok()
        }
        Some(_) | None => None,
    }
}

impl From<QueueTransaction> for TxL1Message {
    fn from(value: QueueTransaction) -> Self {
        TxL1Message {
            queue_index: value.queueIndex,
            gas_limit: value.gasLimit.saturating_to(),
            to: value.target,
            value: value.value,
            sender: value.sender,
            input: value.data,
        }
    }
}
