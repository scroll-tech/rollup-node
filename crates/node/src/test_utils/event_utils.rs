//! Event handling and assertion utilities for test fixtures.

use super::fixture::TestFixture;
use std::time::Duration;

use futures::{FutureExt, StreamExt};
use reth_scroll_primitives::ScrollBlock;
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use rollup_node_primitives::ChainImport;
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
    pub async fn block_sequenced(self, target: u64) -> eyre::Result<ScrollBlock> {
        self.wait_for_event(|e| {
            if let ChainOrchestratorEvent::BlockSequenced(block) = e {
                if block.header.number == target {
                    Some(block.clone())
                } else {
                    None
                }
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
    pub async fn chain_extended(self, target: u64) -> eyre::Result<()> {
        self.wait_for_event(|e| {
            matches!(e, ChainOrchestratorEvent::ChainExtended(ChainImport { chain, .. }) if chain.last().map(|b| b.header.number) == Some(target))
                .then_some(())
        })
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

    /// Wait for any event where the predicate returns true.
    pub async fn where_event(
        self,
        mut predicate: impl FnMut(&ChainOrchestratorEvent) -> bool,
    ) -> eyre::Result<ChainOrchestratorEvent> {
        self.wait_for_event(move |e| if predicate(e) { Some(e.clone()) } else { None }).await
    }

    /// Wait for any event and extract a value from it.
    pub async fn extract<T>(
        self,
        mut extractor: impl FnMut(&ChainOrchestratorEvent) -> Option<T>,
    ) -> eyre::Result<T> {
        self.wait_for_event(move |e| extractor(e)).await
    }

    /// Wait for N events matching a predicate.
    pub async fn where_event_n(
        self,
        count: usize,
        mut predicate: impl FnMut(&ChainOrchestratorEvent) -> bool,
    ) -> eyre::Result<Vec<ChainOrchestratorEvent>> {
        let events = &mut self.fixture.nodes[self.node_index].chain_orchestrator_rx;
        let mut matched_events = Vec::new();

        let result = timeout(self.timeout_duration, async {
            while let Some(event) = events.next().await {
                if predicate(&event) {
                    matched_events.push(event.clone());
                    if matched_events.len() >= count {
                        return Ok(matched_events.clone());
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
                matched_events.len()
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

/// Builder for waiting for events on multiple nodes.
#[derive(Debug)]
pub struct MultiNodeEventWaiter<'a, EC> {
    fixture: &'a mut TestFixture<EC>,
    node_indices: Vec<usize>,
    timeout_duration: Duration,
}

impl<'a, EC> MultiNodeEventWaiter<'a, EC> {
    /// Create a new multi-node event waiter.
    pub fn new(fixture: &'a mut TestFixture<EC>, node_indices: Vec<usize>) -> Self {
        Self { fixture, node_indices, timeout_duration: Duration::from_secs(30) }
    }

    /// Set a custom timeout for waiting.
    pub const fn timeout(mut self, duration: Duration) -> Self {
        self.timeout_duration = duration;
        self
    }

    /// Wait for block sequenced event on all specified nodes.
    pub async fn block_sequenced(self) -> eyre::Result<Vec<ScrollBlock>> {
        self.wait_for_event_on_all(|e| {
            if let ChainOrchestratorEvent::BlockSequenced(block) = e {
                Some(block.clone())
            } else {
                None
            }
        })
        .await
    }

    /// Wait for chain consolidated event on all specified nodes.
    pub async fn chain_consolidated(self) -> eyre::Result<Vec<(u64, u64)>> {
        self.wait_for_event_on_all(|e| {
            if let ChainOrchestratorEvent::ChainConsolidated { from, to } = e {
                Some((*from, *to))
            } else {
                None
            }
        })
        .await
    }

    /// Wait for chain extended event on all specified nodes.
    pub async fn chain_extended(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::ChainExtended(_)).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for chain reorged event on all specified nodes.
    pub async fn chain_reorged(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::ChainReorged(_)).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for L1 synced event on all specified nodes.
    pub async fn l1_synced(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| matches!(e, ChainOrchestratorEvent::L1Synced).then_some(()))
            .await?;
        Ok(())
    }

    /// Wait for optimistic sync event on all specified nodes.
    pub async fn optimistic_sync(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::OptimisticSync(_)).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for new L1 block event on all specified nodes.
    pub async fn new_l1_block(self) -> eyre::Result<Vec<u64>> {
        self.wait_for_event_on_all(|e| {
            if let ChainOrchestratorEvent::NewL1Block(block_number) = e {
                Some(*block_number)
            } else {
                None
            }
        })
        .await
    }

    /// Wait for L1 message committed event on all specified nodes.
    pub async fn l1_message_committed(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::L1MessageCommitted(_)).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for L1 reorg event to be received by all.
    pub async fn l1_reorg(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::L1Reorg { .. }).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for new block received event on all specified nodes.
    pub async fn new_block_received(self) -> eyre::Result<Vec<ScrollBlock>> {
        self.wait_for_event_on_all(|e| {
            if let ChainOrchestratorEvent::NewBlockReceived(block_with_peer) = e {
                Some(block_with_peer.block.clone())
            } else {
                None
            }
        })
        .await
    }

    /// Wait for any event where the predicate returns true on all specified nodes.
    pub async fn where_event(
        self,
        predicate: impl Fn(&ChainOrchestratorEvent) -> bool,
    ) -> eyre::Result<Vec<ChainOrchestratorEvent>> {
        self.wait_for_event_on_all(move |e| if predicate(e) { Some(e.clone()) } else { None }).await
    }

    /// Wait for any event and extract a value from it on all specified nodes.
    pub async fn extract<T>(
        self,
        extractor: impl Fn(&ChainOrchestratorEvent) -> Option<T>,
    ) -> eyre::Result<Vec<T>>
    where
        T: Send + Clone + 'static,
    {
        self.wait_for_event_on_all(extractor).await
    }

    /// Internal helper to wait for a specific event on all nodes.
    async fn wait_for_event_on_all<T>(
        self,
        extractor: impl Fn(&ChainOrchestratorEvent) -> Option<T>,
    ) -> eyre::Result<Vec<T>>
    where
        T: Send + Clone + 'static,
    {
        let timeout_duration = self.timeout_duration;
        let node_indices = self.node_indices;
        let node_count = node_indices.len();

        // Track which nodes have found their event
        let mut results: Vec<Option<T>> = vec![None; node_count];
        let mut completed = 0;

        let result = timeout(timeout_duration, async {
            loop {
                // Poll each node's event stream
                for (idx, &node_index) in node_indices.iter().enumerate() {
                    // Skip nodes that already found their event
                    if results[idx].is_some() {
                        continue;
                    }

                    let events = &mut self.fixture.nodes[node_index].chain_orchestrator_rx;

                    // Try to get the next event (non-blocking with try_next)
                    if let Some(event) = events.next().now_or_never() {
                        match event {
                            Some(event) => {
                                if let Some(value) = extractor(&event) {
                                    results[idx] = Some(value);
                                    completed += 1;

                                    if completed == node_count {
                                        // All nodes have found their events
                                        return Ok(results
                                            .into_iter()
                                            .map(|r| r.unwrap())
                                            .collect::<Vec<T>>());
                                    }
                                }
                            }
                            None => {
                                return Err(eyre::eyre!(
                                    "Event stream ended without matching event on node {}",
                                    node_index
                                ));
                            }
                        }
                    }
                }

                // Small delay to avoid busy waiting
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        })
        .await;

        result.unwrap_or_else(|_| {
            Err(eyre::eyre!(
                "Timeout waiting for event on {} nodes (completed {}/{})",
                node_count,
                completed,
                node_count
            ))
        })
    }
}

/// Extension trait for `TestFixture` to add event waiting capabilities.
pub trait EventAssertions<EC> {
    /// Wait for an event on the sequencer node.
    fn expect_event(&mut self) -> EventWaiter<'_, EC>;

    /// Wait for an event on a specific node.
    fn expect_event_on(&mut self, node_index: usize) -> EventWaiter<'_, EC>;

    /// Wait for an event on multiple nodes.
    fn expect_event_on_nodes(&mut self, node_indices: Vec<usize>) -> MultiNodeEventWaiter<'_, EC>;

    /// Wait for an event on all nodes.
    fn expect_event_on_all_nodes(&mut self) -> MultiNodeEventWaiter<'_, EC>;

    /// Wait for an event on all follower nodes (excluding sequencer at index 0).
    fn expect_event_on_followers(&mut self) -> MultiNodeEventWaiter<'_, EC>;
}

impl<EC> EventAssertions<EC> for TestFixture<EC> {
    fn expect_event(&mut self) -> EventWaiter<'_, EC> {
        EventWaiter::new(self, 0)
    }

    fn expect_event_on(&mut self, node_index: usize) -> EventWaiter<'_, EC> {
        EventWaiter::new(self, node_index)
    }

    fn expect_event_on_nodes(&mut self, node_indices: Vec<usize>) -> MultiNodeEventWaiter<'_, EC> {
        MultiNodeEventWaiter::new(self, node_indices)
    }

    fn expect_event_on_all_nodes(&mut self) -> MultiNodeEventWaiter<'_, EC> {
        let node_indices = (0..self.nodes.len()).collect();
        MultiNodeEventWaiter::new(self, node_indices)
    }

    fn expect_event_on_followers(&mut self) -> MultiNodeEventWaiter<'_, EC> {
        let node_indices = (1..self.nodes.len()).collect();
        MultiNodeEventWaiter::new(self, node_indices)
    }
}
