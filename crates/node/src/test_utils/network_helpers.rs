//! Network-related test helpers for managing peers, reputation, and block propagation.
//!
//! This module provides utilities for testing network-level behaviors including:
//! - Checking and asserting on peer reputation
//! - Announcing blocks to the network
//! - Getting peer information
//!
//! # Examples
//!
//! ## Checking reputation
//!
//! ```rust,ignore
//! use rollup_node::test_utils::{TestFixture, ReputationChecks};
//!
//! #[tokio::test]
//! async fn test_reputation() -> eyre::Result<()> {
//!     let mut fixture = TestFixture::sequencer().with_nodes(2).build().await?;
//!
//!     // Get node 0's peer ID
//!     let node0_peer_id = fixture.network_on(0).peer_id().await?;
//!
//!     // Check reputation from node 1's perspective
//!     fixture.check_reputation_on(1)
//!         .of_peer(node0_peer_id)
//!         .equals(0)
//!         .await?;
//!
//!     // ... do something that should decrease reputation ...
//!
//!     // Wait for reputation to drop below initial value
//!     fixture.check_reputation_on(1)
//!         .of_node(0).await?
//!         .eventually_less_than(0)
//!         .await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Announcing blocks
//!
//! ```rust,ignore
//! use rollup_node::test_utils::{TestFixture, NetworkHelpers};
//! use alloy_primitives::{Signature, U256};
//! use reth_scroll_primitives::ScrollBlock;
//!
//! #[tokio::test]
//! async fn test_block_announce() -> eyre::Result<()> {
//!     let mut fixture = TestFixture::sequencer().with_nodes(2).build().await?;
//!
//!     // Create a block
//!     let block = ScrollBlock::default();
//!     let signature = Signature::new(U256::from(1), U256::from(1), false);
//!
//!     // Announce from node 0
//!     fixture.network_on(0).announce_block(block, signature).await?;
//!
//!     Ok(())
//! }
//! ```

use super::fixture::{ScrollNetworkHandle, TestFixture};
use alloy_primitives::Signature;
use reth_network_api::{PeerId, PeerInfo, Peers};
use reth_scroll_primitives::ScrollBlock;
use std::time::Duration;
use tokio::time;

/// Helper for network-related test operations.
#[derive(Debug)]
pub struct NetworkHelper<'a> {
    fixture: &'a TestFixture,
    node_index: usize,
}

impl<'a> NetworkHelper<'a> {
    /// Create a new network helper for a specific node.
    pub const fn new(fixture: &'a TestFixture, node_index: usize) -> Self {
        Self { fixture, node_index }
    }

    /// Get the network handle for this node.
    pub async fn network_handle(
        &self,
    ) -> eyre::Result<scroll_network::ScrollNetworkHandle<ScrollNetworkHandle>> {
        let node = self.fixture.nodes[self.node_index]
            .as_ref()
            .ok_or_else(|| eyre::eyre!("Node at index {} has been shutdown", self.node_index))?;
        node.rollup_manager_handle
            .get_network_handle()
            .await
            .map_err(|e| eyre::eyre!("Failed to get network handle: {}", e))
    }

    /// Get this node's peer ID.
    pub async fn peer_id(&self) -> eyre::Result<PeerId> {
        let handle = self.network_handle().await?;
        Ok(*handle.inner().peer_id())
    }

    /// Get the reputation of a peer from this node's perspective.
    pub async fn reputation_of(&self, peer_id: PeerId) -> eyre::Result<Option<i32>> {
        let handle = self.network_handle().await?;
        handle
            .inner()
            .reputation_by_id(peer_id)
            .await
            .map_err(|e| eyre::eyre!("Failed to get reputation: {}", e))
    }

    /// Get all connected peers.
    pub async fn peers(&self) -> eyre::Result<Vec<PeerInfo>> {
        let handle = self.network_handle().await?;
        handle.inner().get_all_peers().await.map_err(|e| eyre::eyre!("Failed to get peers: {}", e))
    }

    /// Announce a block from this node to the network.
    pub async fn announce_block(
        &self,
        block: ScrollBlock,
        signature: Signature,
    ) -> eyre::Result<()> {
        let handle = self.network_handle().await?;
        handle.announce_block(block, signature);
        Ok(())
    }

    /// Get the number of connected peers.
    pub async fn peer_count(&self) -> eyre::Result<usize> {
        let peers = self.peers().await?;
        Ok(peers.len())
    }
}

/// Extension trait for `TestFixture` to add network helper capabilities.
pub trait NetworkHelperProvider {
    /// Get a network helper for the sequencer node (node 0).
    fn network(&self) -> NetworkHelper<'_>;

    /// Get a network helper for a specific node by index.
    fn network_on(&self, node_index: usize) -> NetworkHelper<'_>;
}

impl NetworkHelperProvider for TestFixture {
    fn network(&self) -> NetworkHelper<'_> {
        NetworkHelper::new(self, 0)
    }

    fn network_on(&self, node_index: usize) -> NetworkHelper<'_> {
        NetworkHelper::new(self, node_index)
    }
}

/// Builder for checking reputation with assertions.
#[derive(Debug)]
pub struct ReputationChecker<'a> {
    fixture: &'a mut TestFixture,
    observer_node: usize,
    target_peer: Option<PeerId>,
    timeout: Duration,
    poll_interval: Duration,
}

impl<'a> ReputationChecker<'a> {
    /// Create a new reputation checker.
    pub const fn new(fixture: &'a mut TestFixture, observer_node: usize) -> Self {
        Self {
            fixture,
            observer_node,
            target_peer: None,
            timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(100),
        }
    }

    /// Set the target peer by node index.
    pub async fn of_node(mut self, node_index: usize) -> eyre::Result<Self> {
        let peer_id = self.fixture.network_on(node_index).peer_id().await?;
        self.target_peer = Some(peer_id);
        Ok(self)
    }

    /// Set a custom timeout for polling assertions (default: 5s).
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set a custom poll interval for polling assertions (default: 100ms).
    pub const fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Get the current reputation value.
    pub async fn get(&self) -> eyre::Result<Option<i32>> {
        let peer_id = self.target_peer.ok_or_else(|| eyre::eyre!("No target peer set"))?;
        self.fixture.network_on(self.observer_node).reputation_of(peer_id).await
    }

    /// Assert that the reputation equals a specific value.
    pub async fn equals(&mut self, expected: i32) -> eyre::Result<()> {
        let reputation = self.get().await?;
        let actual = reputation.ok_or_else(|| eyre::eyre!("Peer not found"))?;
        if actual != expected {
            return Err(eyre::eyre!("Expected reputation {}, got {}", expected, actual));
        }
        Ok(())
    }

    /// Wait until the reputation becomes less than a threshold.
    /// Polls repeatedly until the condition is met or timeout occurs.
    pub async fn eventually_less_than(self, threshold: i32) -> eyre::Result<()> {
        let peer_id = self.target_peer.ok_or_else(|| eyre::eyre!("No target peer set"))?;
        let mut interval = time::interval(self.poll_interval);
        let start = time::Instant::now();

        loop {
            let reputation =
                self.fixture.network_on(self.observer_node).reputation_of(peer_id).await?;
            if let Some(rep) = reputation {
                if rep < threshold {
                    return Ok(());
                }
            }

            if start.elapsed() > self.timeout {
                let current = reputation.unwrap_or(0);
                return Err(eyre::eyre!(
                    "Timeout waiting for reputation < {} (current: {})",
                    threshold,
                    current
                ));
            }

            interval.tick().await;
        }
    }

    /// Wait until the reputation becomes greater than a threshold.
    /// Polls repeatedly until the condition is met or timeout occurs.
    pub async fn eventually_greater_than(self, threshold: i32) -> eyre::Result<()> {
        let peer_id = self.target_peer.ok_or_else(|| eyre::eyre!("No target peer set"))?;
        let mut interval = time::interval(self.poll_interval);
        let start = time::Instant::now();

        loop {
            let reputation =
                self.fixture.network_on(self.observer_node).reputation_of(peer_id).await?;
            if let Some(rep) = reputation {
                if rep > threshold {
                    return Ok(());
                }
            }

            if start.elapsed() > self.timeout {
                let current = reputation.unwrap_or(0);
                return Err(eyre::eyre!(
                    "Timeout waiting for reputation > {} (current: {})",
                    threshold,
                    current
                ));
            }

            interval.tick().await;
        }
    }
}

/// Extension trait for checking reputation.
pub trait ReputationChecks {
    /// Start checking reputation from the sequencer's perspective.
    fn check_reputation(&mut self) -> ReputationChecker<'_>;

    /// Start checking reputation from a specific node's perspective.
    fn check_reputation_on(&mut self, observer_node: usize) -> ReputationChecker<'_>;
}

impl ReputationChecks for TestFixture {
    fn check_reputation(&mut self) -> ReputationChecker<'_> {
        ReputationChecker::new(self, 0)
    }

    fn check_reputation_on(&mut self, observer_node: usize) -> ReputationChecker<'_> {
        ReputationChecker::new(self, observer_node)
    }
}
