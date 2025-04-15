use alloy_primitives::Log;
use alloy_sol_types::{sol, SolEvent};
use scroll_alloy_consensus::TxL1Message;

sol! {
    #[cfg_attr(feature = "test-utils", derive(arbitrary::Arbitrary))]
    event QueueTransaction(
        address indexed sender,
        address indexed target,
        uint256 value,
        uint64 queueIndex,
        uint256 gasLimit,
        bytes data
    );

    #[cfg_attr(feature = "test-utils", derive(arbitrary::Arbitrary))]
    #[derive(Debug)]
    event CommitBatch(uint256 indexed batch_index, bytes32 indexed batch_hash);

    #[cfg_attr(feature = "test-utils", derive(arbitrary::Arbitrary))]
    #[derive(Debug)]
    event FinalizeBatch(uint256 indexed batch_index, bytes32 indexed batch_hash, bytes32 state_root, bytes32 withdraw_root);
}

/// Tries to decode the provided log into the type T.
pub fn try_decode_log<T: SolEvent>(log: &Log) -> Option<Log<T>> {
    T::decode_log(log).ok()
}

impl From<QueueTransaction> for TxL1Message {
    fn from(value: QueueTransaction) -> Self {
        Self {
            queue_index: value.queueIndex,
            gas_limit: value.gasLimit.saturating_to(),
            to: value.target,
            value: value.value,
            sender: value.sender,
            input: value.data,
        }
    }
}
