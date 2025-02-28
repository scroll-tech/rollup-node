use scroll_alloy_consensus::TxL1Message;

/// A L1 message that is part of the L1 message queue.
#[derive(Debug)]
pub struct L1Message {
    /// The index of the L1 message in the L1 message queue.
    pub queue_index: u64,
    /// The L1 block number at which the L1 message was generated.
    pub block_number: u64,
    /// The L1 transaction.
    pub transaction: TxL1Message,
}
