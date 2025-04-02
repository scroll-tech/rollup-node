//! L2 block and block context implementations.

use std::vec::Vec;

use alloy_primitives::{Bytes, U256};

/// A L2 block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct L2Block {
    /// The RLP-encoded transactions of the block.
    pub transactions: Vec<Bytes>,
    /// The context for the block.
    pub context: BlockContext,
}

impl L2Block {
    /// Returns a new instance of a [`L2Block`].
    pub fn new(transactions: Vec<Bytes>, context: BlockContext) -> Self {
        Self { transactions, context }
    }
}

/// The block's context.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockContext {
    /// The block number.
    pub number: u64,
    /// The block timestamp.
    pub timestamp: u64,
    /// The block base fee.
    pub base_fee: U256,
    /// The block gas limit.
    pub gas_limit: u64,
    /// The block's total transaction count.
    pub num_transactions: u16,
    /// The block's l1 message count.
    pub num_l1_messages: u16,
}

impl BlockContext {
    pub const BYTES_LENGTH: usize = 60;

    /// Returns an owned array which contains all fields of the [`BlockContext`].
    pub fn to_be_bytes(&self) -> [u8; Self::BYTES_LENGTH] {
        let mut buf = [0u8; Self::BYTES_LENGTH];

        buf[..8].copy_from_slice(&self.number.to_be_bytes());
        buf[8..16].copy_from_slice(&self.timestamp.to_be_bytes());
        if self.base_fee != U256::ZERO {
            buf[16..48].copy_from_slice(&self.base_fee.to_be_bytes::<32>());
        }
        buf[48..56].copy_from_slice(&self.gas_limit.to_be_bytes());
        buf[56..58].copy_from_slice(&self.num_transactions.to_be_bytes());
        buf[58..].copy_from_slice(&self.num_l1_messages.to_be_bytes());
        buf
    }
}
