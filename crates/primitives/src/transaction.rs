use scroll_alloy_consensus::TxL1Message;

/// A L1 message that is part of the L1 message queue.
#[derive(Debug)]
pub struct L1MessageWithBlockNumber {
    /// The L1 block number at which the L1 message was generated.
    pub block_number: u64,
    /// The L1 transaction.
    pub transaction: TxL1Message,
}

impl L1MessageWithBlockNumber {
    /// Returns a new [`L1MessageWithBlockNumber`].
    pub fn new(block_number: u64, transaction: TxL1Message) -> Self {
        Self { block_number, transaction }
    }
}
