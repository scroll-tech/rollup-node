//! Event handling and assertion utilities for test fixtures.

use super::fixture::TestFixture;
use std::time::Duration;

use futures::{FutureExt, StreamExt};
use reth_scroll_primitives::ScrollBlock;
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use rollup_node_primitives::ChainImport;
use tokio::time::timeout;

/// The default event wait time.
pub const DEFAULT_EVENT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Builder for waiting for events on multiple nodes.
#[derive(Debug)]
pub struct EventWaiter<'a> {
    fixture: &'a mut TestFixture,
    node_indices: Vec<usize>,
    timeout_duration: Duration,
}

impl<'a> EventWaiter<'a> {
    /// Create a new multi-node event waiter.
    pub const fn new(fixture: &'a mut TestFixture, node_indices: Vec<usize>) -> Self {
        Self { fixture, node_indices, timeout_duration: Duration::from_secs(30) }
    }

    /// Set a custom timeout for waiting.
    pub const fn timeout(mut self, duration: Duration) -> Self {
        self.timeout_duration = duration;
        self
    }

    /// Wait for block sequenced event on all specified nodes.
    pub async fn block_sequenced(self, target: u64) -> eyre::Result<ScrollBlock> {
        self.wait_for_event_on_all(|e| {
            if let ChainOrchestratorEvent::BlockSequenced(block) = e {
                (block.header.number == target).then(|| block.clone())
            } else {
                None
            }
        })
        .await
        .map(|v| v.first().expect("should have block sequenced").clone())
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
    pub async fn chain_extended(self, target: u64) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::ChainExtended(ChainImport{chain,..}) if chain.last().map(|b| b.header.number) >= Some(target)).then_some(())
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

    /// Wait for batch commit indexed event on all specified nodes.
    pub async fn batch_commit_indexed(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::BatchCommitIndexed { .. }).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for batch consolidated event on all specified nodes.
    pub async fn batch_consolidated(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::BatchConsolidated(_)).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for block consolidated event on all specified nodes.
    pub async fn block_consolidated(self, target_block: u64) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            if let ChainOrchestratorEvent::BlockConsolidated(outcome) = e {
                (outcome.block_info().block_info.number == target_block).then_some(())
            } else {
                None
            }
        })
        .await?;
        Ok(())
    }

    /// Wait for batch finalize indexed event on all specified nodes.
    pub async fn batch_finalize_indexed(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::BatchFinalizeIndexed { .. }).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for batch reverted event on all specified nodes.
    pub async fn batch_reverted(self) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::BatchReverted { .. }).then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for L1 block finalized event on all specified nodes.
    pub async fn l1_block_finalized(self) -> eyre::Result<()> {
        self.l1_block_finalized_at_least(0).await
    }

    /// Wait for L1 block finalized event on all specified nodes where the block number
    /// is at least the specified target.
    pub async fn l1_block_finalized_at_least(self, target_block_number: u64) -> eyre::Result<()> {
        self.wait_for_event_on_all(|e| {
            matches!(e, ChainOrchestratorEvent::L1BlockFinalized(n, _) if n >= &target_block_number)
                .then_some(())
        })
        .await?;
        Ok(())
    }

    /// Wait for new block received event on all specified nodes.
    pub async fn new_block_received(self) -> eyre::Result<ScrollBlock> {
        self.wait_for_event_on_all(|e| {
            if let ChainOrchestratorEvent::NewBlockReceived(block_with_peer) = e {
                Some(block_with_peer.block.clone())
            } else {
                None
            }
        })
        .await
        .map(|v| v.first().expect("should have block received").clone())
    }

    /// Wait for any event where the predicate returns true on all specified nodes.
    pub async fn where_event(
        self,
        predicate: impl Fn(&ChainOrchestratorEvent) -> bool,
    ) -> eyre::Result<Vec<ChainOrchestratorEvent>> {
        self.wait_for_event_on_all(move |e| predicate(e).then(|| e.clone())).await
    }

    /// Wait for N events matching a predicate.
    pub async fn where_n_events(
        self,
        count: usize,
        mut predicate: impl FnMut(&ChainOrchestratorEvent) -> bool,
    ) -> eyre::Result<Vec<ChainOrchestratorEvent>> {
        let mut matched_events = Vec::new();
        for node in self.node_indices {
            let Some(node_handle) = &mut self.fixture.nodes[node] else {
                continue; // Skip shutdown nodes
            };
            let events = &mut node_handle.chain_orchestrator_rx;
            let mut node_matched_events = Vec::new();

            let result = timeout(self.timeout_duration, async {
                while let Some(event) = events.next().await {
                    if predicate(&event) {
                        node_matched_events.push(event.clone());
                        if node_matched_events.len() >= count {
                            return Ok(matched_events.clone());
                        }
                    }
                }
                Err(eyre::eyre!("Event stream ended before matching {} events", count))
            })
            .await;

            match result {
                Ok(_) => matched_events = node_matched_events,
                Err(_) => {
                    return Err(eyre::eyre!(
                        "Timeout waiting for {} events (matched {} so far)",
                        count,
                        matched_events.len()
                    ))
                }
            }
        }

        Ok(matched_events)
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

                    let node_handle = self.fixture.nodes[node_index].as_mut().ok_or_else(|| {
                        eyre::eyre!("Node at index {} has been shutdown", node_index)
                    })?;
                    let events = &mut node_handle.chain_orchestrator_rx;

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
pub trait EventAssertions {
    /// Wait for an event on the sequencer node.
    fn expect_event(&mut self) -> EventWaiter<'_>;

    /// Wait for an event on a specific node.
    fn expect_event_on(&mut self, node_index: usize) -> EventWaiter<'_>;

    /// Wait for an event on multiple nodes.
    fn expect_event_on_nodes(&mut self, node_indices: Vec<usize>) -> EventWaiter<'_>;

    /// Wait for an event on all nodes.
    fn expect_event_on_all_nodes(&mut self) -> EventWaiter<'_>;

    /// Wait for an event on all follower nodes (excluding sequencer at index 0).
    fn expect_event_on_followers(&mut self) -> EventWaiter<'_>;
}

impl EventAssertions for TestFixture {
    fn expect_event(&mut self) -> EventWaiter<'_> {
        EventWaiter::new(self, vec![0])
    }

    fn expect_event_on(&mut self, node_index: usize) -> EventWaiter<'_> {
        EventWaiter::new(self, vec![node_index])
    }

    fn expect_event_on_nodes(&mut self, node_indices: Vec<usize>) -> EventWaiter<'_> {
        EventWaiter::new(self, node_indices)
    }

    fn expect_event_on_all_nodes(&mut self) -> EventWaiter<'_> {
        let node_indices = (0..self.nodes.len()).collect();
        EventWaiter::new(self, node_indices)
    }

    fn expect_event_on_followers(&mut self) -> EventWaiter<'_> {
        let node_indices = (1..self.nodes.len()).collect();
        EventWaiter::new(self, node_indices)
    }
}
