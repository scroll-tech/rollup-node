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
    /// The block's l1 message count.
    pub num_l1_messages: u16,
}
