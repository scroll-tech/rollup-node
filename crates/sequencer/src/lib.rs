//! Sequencer is a library that
use futures::Stream;
use std::collections::VecDeque;
use tokio::time::Interval;

use reth_scroll_primitives::ScrollBlock;
use scroll_primitives::L1Message;

/// The sequencer is responsible for creating new [`ScrollBlock`]s.
#[derive(Debug)]
pub struct Sequencer {
    // TODO: Replace with appropriate buffer type that is reorg aware.
    /// A transaction queue for L1 messages.
    tx_queue: VecDeque<L1Message>,
    /// The interval at which the sequencer creates new [`ScrollBlock`]s.
    block_interval: Interval,
}

impl Sequencer {
    /// Creates a new [`Sequencer`] instance.
    pub fn new(tx_queue: impl IntoIterator<Item = L1Message>, block_interval: Interval) -> Self {
        Self { tx_queue: tx_queue.into_iter().collect(), block_interval }
    }

    /// Creates a new [`ScrollBlock`] based on the current state of the sequencer.
    pub fn new_block(&mut self) -> ScrollBlock {
        todo!()
    }

    /// Handles a L1 message.
    pub fn handle_l1_message(&mut self, _l1_message: L1Message) {
        todo!()
    }

    /// Handles a reorg at the provided block number.
    pub fn handle_reorg(&mut self, _block_number: u64) {
        todo!()
    }
}

impl Stream for Sequencer {
    type Item = ScrollBlock;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if let std::task::Poll::Ready(_) = this.block_interval.poll_tick(cx) {
            let block = this.new_block();
            std::task::Poll::Ready(Some(block))
        } else {
            std::task::Poll::Pending
        }
    }
}
