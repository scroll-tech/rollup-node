//! Event handling and assertion utilities for test fixtures.

use super::fixture::TestFixture;
use futures::StreamExt;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use std::time::Duration;
use tokio::time::timeout;

/// Builder for waiting for specific events with optional assertions.
#[derive(Debug)]
pub struct EventWaiter<'a, EC> {
    fixture: &'a mut TestFixture<EC>,
    node_index: usize,
    timeout_duration: Duration,
}

impl<'a, EC> EventWaiter<'a, EC> {
    /// Create a new event waiter.
    pub fn new(fixture: &'a mut TestFixture<EC>, node_index: usize) -> Self {
        Self { fixture, node_index, timeout_duration: Duration::from_secs(30) }
    }

    /// Set a custom timeout for waiting.
    pub const fn timeout(mut self, duration: Duration) -> Self {
        self.timeout_duration = duration;
        self
    }

    /// Wait for a block sequenced event.
    pub async fn block_sequenced(self) -> eyre::Result<ScrollBlock> {
        self.wait_for_event(|e| {
            if let ChainOrchestratorEvent::BlockSequenced(block) = e {
                Some(block.clone())
            } else {
                None
            }
        })
        .await
    }

    /// Wait for a chain consolidated event.
    pub async fn chain_consolidated(self) -> eyre::Result<(u64, u64)> {
        self.wait_for_event(|e| {
            if let ChainOrchestratorEvent::ChainConsolidated { from, to } = e {
                Some((*from, *to))
            } else {
                None
            }
        })
        .await
    }

    /// Wait for a chain extended event.
    pub async fn chain_extended(self) -> eyre::Result<()> {
        self.wait_for_event(|e| matches!(e, ChainOrchestratorEvent::ChainExtended(_)).then_some(()))
            .await
    }

    /// Wait for a chain reorged event.
    pub async fn chain_reorged(self) -> eyre::Result<()> {
        self.wait_for_event(|e| matches!(e, ChainOrchestratorEvent::ChainReorged(_)).then_some(()))
            .await
    }

    /// Wait for an L1 synced event.
    pub async fn l1_synced(self) -> eyre::Result<()> {
        self.wait_for_event(|e| matches!(e, ChainOrchestratorEvent::L1Synced).then_some(())).await
    }

    /// Wait for an optimistic sync event.
    pub async fn optimistic_sync(self) -> eyre::Result<()> {
        self.wait_for_event(|e| {
            matches!(e, ChainOrchestratorEvent::OptimisticSync(_)).then_some(())
        })
        .await
    }

    /// Wait for a new L1 block event.
    pub async fn new_l1_block(self) -> eyre::Result<u64> {
        self.wait_for_event(|e| {
            if let ChainOrchestratorEvent::NewL1Block(block_number) = e {
                Some(*block_number)
            } else {
                None
            }
        })
        .await
    }

    /// Wait for an L1 message committed event.
    pub async fn l1_message_committed(self) -> eyre::Result<()> {
        self.wait_for_event(|e| {
            matches!(e, ChainOrchestratorEvent::L1MessageCommitted(_)).then_some(())
        })
        .await
    }

    /// Wait for a new block received event from the network.
    pub async fn new_block_received(self) -> eyre::Result<ScrollBlock> {
        self.wait_for_event(|e| {
            if let ChainOrchestratorEvent::NewBlockReceived(block_with_peer) = e {
                Some(block_with_peer.block.clone())
            } else {
                None
            }
        })
        .await
    }

    /// Wait for any event matching a custom predicate.
    pub async fn matching<T>(
        self,
        mut predicate: impl FnMut(&ChainOrchestratorEvent) -> Option<T>,
    ) -> eyre::Result<T> {
        self.wait_for_event(move |e| predicate(e)).await
    }

    /// Wait for N events matching a predicate.
    pub async fn matching_n<T>(
        self,
        count: usize,
        mut predicate: impl FnMut(&ChainOrchestratorEvent) -> bool,
    ) -> eyre::Result<()> {
        let events = &mut self.fixture.nodes[self.node_index].chain_orchestrator_rx;
        let mut matched = 0;

        let result = timeout(self.timeout_duration, async {
            while let Some(event) = events.next().await {
                if predicate(&event) {
                    matched += 1;
                    if matched >= count {
                        return Ok(());
                    }
                }
            }
            Err(eyre::eyre!("Event stream ended before matching {} events", count))
        })
        .await;

        match result {
            Ok(r) => r,
            Err(_) => Err(eyre::eyre!(
                "Timeout waiting for {} events (matched {} so far)",
                count,
                matched
            )),
        }
    }

    /// Internal helper to wait for a specific event.
    async fn wait_for_event<T>(
        self,
        mut extractor: impl FnMut(&ChainOrchestratorEvent) -> Option<T>,
    ) -> eyre::Result<T> {
        let events = &mut self.fixture.nodes[self.node_index].chain_orchestrator_rx;

        let result = timeout(self.timeout_duration, async {
            while let Some(event) = events.next().await {
                if let Some(value) = extractor(&event) {
                    return Ok(value);
                }
            }
            Err(eyre::eyre!("Event stream ended without matching event"))
        })
        .await;

        result.unwrap_or_else(|_| Err(eyre::eyre!("Timeout waiting for event")))
    }
}

/// Fluent builder for chaining multiple event expectations.
#[derive(Debug)]
pub struct EventSequence<'a, EC> {
    fixture: &'a mut TestFixture<EC>,
    node_index: usize,
    timeout_duration: Duration,
}

impl<'a, EC> EventSequence<'a, EC> {
    /// Create a new event sequence.
    pub fn new(fixture: &'a mut TestFixture<EC>, node_index: usize) -> Self {
        Self { fixture, node_index, timeout_duration: Duration::from_secs(30) }
    }

    /// Set a custom timeout for all events in the sequence.
    pub const fn timeout(mut self, duration: Duration) -> Self {
        self.timeout_duration = duration;
        self
    }

    /// Wait for L1 synced, then continue the sequence.
    pub async fn l1_synced(self) -> eyre::Result<Self> {
        EventWaiter::new(self.fixture, self.node_index)
            .timeout(self.timeout_duration)
            .l1_synced()
            .await?;
        Ok(self)
    }

    /// Wait for chain consolidated, then continue the sequence.
    pub async fn then_chain_consolidated(self) -> eyre::Result<Self> {
        EventWaiter::new(self.fixture, self.node_index)
            .timeout(self.timeout_duration)
            .chain_consolidated()
            .await?;
        Ok(self)
    }

    /// Wait for chain extended, then continue the sequence.
    pub async fn then_chain_extended(self) -> eyre::Result<Self> {
        EventWaiter::new(self.fixture, self.node_index)
            .timeout(self.timeout_duration)
            .chain_extended()
            .await?;
        Ok(self)
    }

    /// Wait for block sequenced, then continue the sequence.
    pub async fn then_block_sequenced(self) -> eyre::Result<Self> {
        EventWaiter::new(self.fixture, self.node_index)
            .timeout(self.timeout_duration)
            .block_sequenced()
            .await?;
        Ok(self)
    }
}

/// Extension trait for `TestFixture` to add event waiting capabilities.
pub trait EventAssertions<EC> {
    /// Wait for an event on the sequencer node.
    fn expect_event(&mut self) -> EventWaiter<'_, EC>;

    /// Wait for an event on a specific node.
    fn expect_event_on(&mut self, node_index: usize) -> EventWaiter<'_, EC>;

    /// Wait for a sequence of events on the sequencer node.
    fn expect_events(&mut self) -> EventSequence<'_, EC>;

    /// Wait for a sequence of events on a specific node.
    fn expect_events_on(&mut self, node_index: usize) -> EventSequence<'_, EC>;
}

impl<EC> EventAssertions<EC> for TestFixture<EC> {
    fn expect_event(&mut self) -> EventWaiter<'_, EC> {
        EventWaiter::new(self, 0)
    }

    fn expect_event_on(&mut self, node_index: usize) -> EventWaiter<'_, EC> {
        EventWaiter::new(self, node_index)
    }

    fn expect_events(&mut self) -> EventSequence<'_, EC> {
        EventSequence::new(self, 0)
    }

    fn expect_events_on(&mut self, node_index: usize) -> EventSequence<'_, EC> {
        EventSequence::new(self, node_index)
    }
}
