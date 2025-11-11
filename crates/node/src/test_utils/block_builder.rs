//! Block building helpers for test fixtures.

use super::fixture::TestFixture;
use crate::test_utils::EventAssertions;

use alloy_primitives::B256;
use reth_primitives_traits::transaction::TxHashRef;
use reth_scroll_primitives::ScrollBlock;
use scroll_alloy_consensus::ScrollTransaction;

/// Builder for constructing and validating blocks in tests.
#[derive(Debug)]
pub struct BlockBuilder<'a> {
    fixture: &'a mut TestFixture,
    expected_tx_hashes: Vec<B256>,
    expected_tx_count: Option<usize>,
    expected_base_fee: Option<u64>,
    expected_block_number: Option<u64>,
    expect_l1_message: bool,
    expected_l1_message_count: Option<usize>,
}

impl<'a> BlockBuilder<'a> {
    /// Create a new block builder.
    pub(crate) fn new(fixture: &'a mut TestFixture) -> Self {
        Self {
            fixture,
            expected_tx_hashes: Vec::new(),
            expected_tx_count: None,
            expected_block_number: None,
            expected_base_fee: None,
            expect_l1_message: false,
            expected_l1_message_count: None,
        }
    }

    /// Expect a specific transaction to be included in the block.
    pub fn expect_tx(mut self, tx_hash: B256) -> Self {
        self.expected_tx_hashes.push(tx_hash);
        self
    }

    /// Expect a specific number of transactions in the block.
    pub const fn expect_tx_count(mut self, count: usize) -> Self {
        self.expected_tx_count = Some(count);
        self
    }

    /// Expect a specific block number.
    pub const fn expect_block_number(mut self, number: u64) -> Self {
        self.expected_block_number = Some(number);
        self
    }

    /// Expect at least one L1 message in the block.
    pub const fn expect_l1_message(mut self) -> Self {
        self.expect_l1_message = true;
        self
    }

    /// Expect a specific number of L1 messages in the block.
    pub const fn expect_l1_message_count(mut self, count: usize) -> Self {
        self.expected_l1_message_count = Some(count);
        self
    }

    /// Build the block and validate against expectations.
    pub async fn await_block(self) -> eyre::Result<ScrollBlock> {
        let sequencer_node = &self.fixture.nodes[0];

        // Get the sequencer from the rollup manager handle
        let handle = &sequencer_node.rollup_manager_handle;

        // Trigger block building
        handle.build_block();

        // If extract the block number.
        let expect = self.fixture.expect_event();
        let block =
            if let Some(b) = self.expected_block_number {
                expect.block_sequenced(b).await?
            } else {
                expect.extract(|e| {
                if let rollup_node_chain_orchestrator::ChainOrchestratorEvent::BlockSequenced(
                    block,
                ) = e
                {
                    Some(block.clone())
                } else {
                    None
                }
            }).await?
            };

        // Finally validate the block.
        self.validate_block(&block)
    }

    /// Validate the block against expectations.
    fn validate_block(self, block: &ScrollBlock) -> eyre::Result<ScrollBlock> {
        // Check transaction count
        if let Some(expected_count) = self.expected_tx_count {
            if block.body.transactions.len() != expected_count {
                return Err(eyre::eyre!(
                    "Expected {} transactions, but block has {}",
                    expected_count,
                    block.body.transactions.len()
                ));
            }
        }

        // Check block number
        if let Some(expected_number) = self.expected_block_number {
            if block.header.number != expected_number {
                return Err(eyre::eyre!(
                    "Expected {} number, but block has {}",
                    expected_number,
                    block.header.number
                ));
            }
        }

        // Check specific transaction hashes
        for expected_hash in &self.expected_tx_hashes {
            if !block.body.transactions.iter().any(|tx| tx.tx_hash() == expected_hash) {
                return Err(eyre::eyre!(
                    "Expected transaction {:?} not found in block",
                    expected_hash
                ));
            }
        }

        // Check base fee
        if let Some(expected_base_fee) = self.expected_base_fee {
            let actual_base_fee = block
                .header
                .base_fee_per_gas
                .ok_or_else(|| eyre::eyre!("Block has no base fee"))?;
            if actual_base_fee != expected_base_fee {
                return Err(eyre::eyre!(
                    "Expected base fee {}, but block has {}",
                    expected_base_fee,
                    actual_base_fee
                ));
            }
        }

        // Check L1 messages
        if self.expect_l1_message {
            let l1_message_count =
                block.body.transactions.iter().filter(|tx| tx.queue_index().is_some()).count();
            if l1_message_count == 0 {
                return Err(eyre::eyre!("Expected at least one L1 message, but block has none"));
            }
        }

        if let Some(expected_count) = self.expected_l1_message_count {
            let l1_message_count =
                block.body.transactions.iter().filter(|tx| tx.queue_index().is_some()).count();
            if l1_message_count != expected_count {
                return Err(eyre::eyre!(
                    "Expected {} L1 messages, but block has {}",
                    expected_count,
                    l1_message_count
                ));
            }
        }

        Ok(block.clone())
    }
}
