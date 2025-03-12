//! A library responsible for indexing data relevant to the L1.

use scroll_l1::L1Event;
use scroll_primitives::{BatchInput, L1Message};
use std::sync::Arc;

/// The indexer is responsible for indexing data relevant to the L1.
#[derive(Debug)]
pub struct Indexer;

impl Indexer {
    /// Handles an event from the L1.
    pub async fn handle_l1_event(&mut self, event: L1Event) {
        match event {
            L1Event::CommitBatch(batch_input) => self.handle_batch_input(batch_input).await,
            L1Event::Reorg(block_number) => self.handle_reorg(block_number).await,
            L1Event::NewBlock(_block_number) => todo!(),
            L1Event::Finalized(block_number) => self.handle_finalized(block_number).await,
            L1Event::L1Message(l1_message) => self.handle_l1_message(l1_message).await,
        }
    }

    async fn handle_reorg(&self, _block_number: u64) {
        todo!()
    }

    async fn handle_finalized(&self, _block_number: u64) {
        todo!()
    }

    async fn handle_l1_message(&self, _l1_message: Arc<L1Message>) {
        todo!()
    }

    async fn handle_batch_input(&self, _batch_input: Arc<BatchInput>) {
        todo!()
    }
}
