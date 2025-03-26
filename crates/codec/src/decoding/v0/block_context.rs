use crate::{from_be_bytes_slice_and_advance_buf, BlockContext};

use alloy_primitives::{bytes::Buf, U256};

#[derive(Debug)]
pub(crate) struct BlockContextV0 {
    pub(crate) number: u64,
    pub(crate) timestamp: u64,
    pub(crate) base_fee: U256,
    pub(crate) gas_limit: u64,
    pub(crate) num_transactions: u16,
    pub(crate) num_l1_messages: u16,
}

impl BlockContextV0 {
    pub(crate) const BYTES_LENGTH: usize = 60;

    /// Tries to read from the input buffer into the [`BlockContextV0`].
    /// Returns [`None`] if the buffer.len() < [`BlockContextV0::BYTES_LENGTH`].
    pub(crate) fn try_from_buf(buf: &mut &[u8]) -> Option<Self> {
        if buf.len() < Self::BYTES_LENGTH {
            return None
        }
        let number = from_be_bytes_slice_and_advance_buf!(u64, buf);
        let timestamp = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let base_fee = U256::from_be_slice(&buf[0..32]);
        buf.advance(32);

        let gas_limit = from_be_bytes_slice_and_advance_buf!(u64, buf);
        let num_transactions = from_be_bytes_slice_and_advance_buf!(u16, buf);
        let num_l1_messages = from_be_bytes_slice_and_advance_buf!(u16, buf);

        Some(Self { number, timestamp, base_fee, gas_limit, num_transactions, num_l1_messages })
    }

    /// Returns the L2 transaction count for the block, excluding L1 messages.
    pub(crate) fn transactions_count(&self) -> usize {
        self.num_transactions.saturating_sub(self.num_l1_messages) as usize
    }
}

impl From<BlockContextV0> for BlockContext {
    fn from(value: BlockContextV0) -> Self {
        Self {
            number: value.number,
            timestamp: value.timestamp,
            base_fee: value.base_fee,
            gas_limit: value.gas_limit,
            num_transactions: value.num_transactions,
            num_l1_messages: value.num_l1_messages,
        }
    }
}
