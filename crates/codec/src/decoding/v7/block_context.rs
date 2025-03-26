use crate::{from_be_bytes_slice_and_advance_buf, BlockContext};

use alloy_primitives::{bytes::Buf, U256};

#[derive(Debug)]
pub(crate) struct BlockContextV7 {
    pub(crate) timestamp: u64,
    pub(crate) base_fee: U256,
    pub(crate) gas_limit: u64,
    pub(crate) num_transactions: u16,
    pub(crate) num_l1_messages: u16,
}

impl BlockContextV7 {
    pub(crate) const BYTES_LENGTH: usize = 52;

    /// Tries to read from the input buffer into the [`BlockContextV7`].
    /// Returns [`None`] if the buffer.len() < [`BlockContextV7::BYTES_LENGTH`].
    pub(crate) fn try_from_buf(buf: &mut &[u8]) -> Option<Self> {
        if buf.len() < Self::BYTES_LENGTH {
            return None
        }
        let timestamp = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let base_fee = U256::from_be_slice(&buf[0..32]);
        buf.advance(32);

        let gas_limit = from_be_bytes_slice_and_advance_buf!(u64, buf);
        let num_transactions = from_be_bytes_slice_and_advance_buf!(u16, buf);
        let num_l1_messages = from_be_bytes_slice_and_advance_buf!(u16, buf);

        Some(Self { timestamp, base_fee, gas_limit, num_transactions, num_l1_messages })
    }

    /// Returns the L2 transaction count for the block, excluding L1 messages.
    pub(crate) fn transactions_count(&self) -> usize {
        self.num_transactions.saturating_sub(self.num_l1_messages) as usize
    }
}

impl From<(BlockContextV7, u64)> for BlockContext {
    fn from((context, block_number): (BlockContextV7, u64)) -> Self {
        Self {
            number: block_number,
            timestamp: context.timestamp,
            base_fee: context.base_fee,
            gas_limit: context.gas_limit,
            num_transactions: context.num_transactions,
            num_l1_messages: context.num_l1_messages,
        }
    }
}
