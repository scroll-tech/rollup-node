use alloy_primitives::B256;
use scroll_alloy_consensus::TxL1Message;

/// A L1 message envelope, containing extra information about the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L1MessageEnvelope {
    /// The L1 transaction.
    pub transaction: TxL1Message,
    /// The L1 block number at which the L1 message was generated.
    pub l1_block_number: u64,
    /// The L2 block number at which the L1 message was processed.
    pub l2_block_number: Option<u64>,
    /// The queue hash for the message.
    pub queue_hash: Option<B256>,
}

impl L1MessageEnvelope {
    /// Returns a new [`L1MessageEnvelope`].
    pub const fn new(
        transaction: TxL1Message,
        l1_block_number: u64,
        l2_block_number: Option<u64>,
        queue_hash: Option<B256>,
    ) -> Self {
        Self { l1_block_number, transaction, l2_block_number, queue_hash }
    }
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for L1MessageEnvelope {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(Self {
            l1_block_number: u.arbitrary::<u32>()? as u64,
            l2_block_number: u.arbitrary::<Option<u32>>()?.map(|x| x as u64),
            queue_hash: Some(u.arbitrary::<B256>()?),
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
