use scroll_alloy_consensus::TxL1Message;

/// A L1 message that is part of the L1 message queue.
#[derive(Debug, Clone, PartialEq)]
pub struct L1Message {
    /// The L1 block number at which the L1 message was generated.
    pub block_number: u64,
    /// The L1 transaction.
    pub transaction: TxL1Message,
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for L1Message {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(Self {
            block_number: u.arbitrary::<u32>()? as u64,
            transaction: TxL1Message {
                queue_index: u.arbitrary::<u32>()? as u64,
                gas_limit: u.arbitrary()?,
                to: u.arbitrary()?,
                value: u.arbitrary()?,
                sender: u.arbitrary()?,
                input: u.arbitrary()?,
            },
        })
    }
}
