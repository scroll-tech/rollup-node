//! Reboot utilities for the rollup node.
//!
//! This module provides functionality to safely shutdown and restart nodes in integration tests,
//! enabling testing of:
//! - State persistence across reboots
//! - Correct restoration of `ForkchoiceState` (FCS) from database
//! - Continued L1 event processing after restart
//! - Proper handling of L1 reorgs after node restarts
//!
//! # Key Features
//!
//! ## Explicit Shutdown
//! The `shutdown_node()` method performs a graceful shutdown by:
//! 1. Sending an explicit shutdown command to the `ChainOrchestrator`
//! 2. Dropping the `NodeHandle` to release all resources
//! 3. Waiting for async cleanup to complete
//!
//! This ensures the node stops cleanly without lingering RPC errors or background tasks.
//!
//! ## State Restoration
//! The `start_node()` method restarts a previously shutdown node by:
//! 1. Reusing the existing database (state is preserved)
//! 2. Restoring the `ForkchoiceState` from the `ChainOrchestrator`'s persisted status
//! 3. Initializing the `Engine` with the restored FCS (not genesis)
//!
//! This prevents "Syncing" errors that would occur if the FCS was reset to genesis
//! while the execution client's state remained at a later block.
//!
//! # Example
//! ```ignore
//! let mut fixture = TestFixture::builder()
//!     .followers(1)
//!     .with_anvil(None, None, Some(22222222), None, None)
//!     .build()
//!     .await?;
//!
//! // Process some L1 events
//! fixture.l1().sync().await?;
//! for i in 0..=3 {
//!     let tx = read_test_transaction("commitBatch", &i.to_string())?;
//!     fixture.anvil_inject_tx(tx).await?;
//! }
//!
//! // Reboot the node
//! fixture.shutdown_node(0).await?;
//! fixture.start_node(0).await?;
//!
//! // Node continues processing from where it left off
//! fixture.l1().sync().await?;
//! fixture.expect_event().l1_synced().await?;
//! ```

use crate::test_utils::{setup_engine, NodeHandle, TestFixture};
use scroll_alloy_provider::{ScrollAuthApiEngineClient, ScrollEngineApi};
use scroll_engine::Engine;
use std::sync::Arc;

impl TestFixture {
    /// Shutdown a node by index, performing a graceful shutdown of all components.
    ///
    /// # Shutdown Process
    ///
    /// 1. **Explicit `ChainOrchestrator` Shutdown**
    ///    - Sends a `Shutdown` command to the `ChainOrchestrator` via its handle
    ///    - This makes the orchestrator exit its event loop immediately
    ///    - Prevents lingering RPC errors (e.g., 502 errors) after shutdown
    ///
    /// 2. **Drop `NodeHandle`**
    ///    - Takes ownership of the `NodeHandle` and drops it
    ///    - Triggers cleanup of:
    ///      - Network connections
    ///      - RPC server (stops listening on port)
    ///      - Database connections (flushes pending writes)
    ///      - Background tasks (via `TaskManager` shutdown signals)
    ///      - L1 watcher channels
    ///
    /// 3. **Wait for Cleanup**
    ///    - Sleeps for 1 second to allow async cleanup to complete
    ///    - Ensures database WAL is checkpointed (in WAL mode)
    ///    - Allows all background tasks to finish gracefully
    ///
    /// The node can later be restarted using `start_node()`, which will reuse
    /// the existing database and restore state from it.
    ///
    /// # Arguments
    /// * `node_index` - Index of the node to shutdown (0 = sequencer if enabled)
    ///
    /// # Errors
    /// Returns an error if:
    /// - `node_index` is out of bounds
    /// - The node at `node_index` is already shutdown
    ///
    /// # Example
    /// ```ignore
    /// let mut fixture = TestFixture::builder().followers(2).build().await?;
    ///
    /// // Shutdown follower node at index 1
    /// fixture.shutdown_node(1).await?;
    ///
    /// // Node slot is now None, but indices are preserved
    /// assert!(fixture.nodes[1].is_none());
    ///
    /// // Later, restart the node
    /// fixture.start_node(1).await?;
    /// ```
    pub async fn shutdown_node(&mut self, node_index: usize) -> eyre::Result<()> {
        if node_index >= self.nodes.len() {
            return Err(eyre::eyre!(
                "Node index {} out of bounds (total nodes: {})",
                node_index,
                self.nodes.len()
            ));
        }

        if self.nodes[node_index].is_none() {
            return Err(eyre::eyre!("Node at index {} is already shutdown", node_index));
        }

        tracing::info!("Shutting down node at index {}", node_index);

        // Step 1: Explicitly shutdown the ChainOrchestrator
        // This sends a Shutdown command that will make the ChainOrchestrator exit its event loop
        // immediately
        if let Some(node) = &self.nodes[node_index] {
            tracing::info!("Sending shutdown command to ChainOrchestrator...");
            if let Err(e) = node.rollup_manager_handle.shutdown().await {
                tracing::warn!("Failed to send shutdown command to ChainOrchestrator: {}", e);
            } else {
                tracing::info!("ChainOrchestrator shutdown command acknowledged");
            }
        }

        // Step 2: Take ownership and drop the node handle
        // This closes all channels (RPC, network, DB) and releases resources
        let node_handle = self.nodes[node_index].take();
        drop(node_handle);

        // Step 3: Wait for async cleanup to complete
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        tracing::info!("Node at index {} shutdown complete", node_index);
        Ok(())
    }

    /// Restart a previously shutdown node at the given index.
    ///
    /// This method restarts a node that was shut down, reusing its existing database
    /// and restoring its state (including `ForkchoiceState`) from persisted data.
    ///
    /// # State Restoration
    ///
    /// The restarted node will:
    /// 1. Reuse the same database (no data loss)
    /// 2. Restore the `ForkchoiceState` from `ChainOrchestrator`'s persisted status
    /// 3. Initialize the `Engine` with the restored FCS (not genesis)
    /// 4. Continue processing L1 events from where it left off
    ///
    /// This ensures the node's view of the chain is consistent with the execution
    /// client's actual state, preventing "Syncing" errors when building payloads.
    ///
    /// # Arguments
    /// * `node_index` - Index of the node to start (0 = sequencer if enabled)
    ///
    /// # Errors
    /// Returns an error if:
    /// - `node_index` is out of bounds
    /// - The node at `node_index` is already running
    ///
    /// # Example
    /// ```ignore
    /// let mut fixture = TestFixture::builder().followers(1).build().await?;
    ///
    /// // Shutdown the follower
    /// fixture.shutdown_node(0).await?;
    ///
    /// // Restart the follower (reuses database and restores state)
    /// fixture.start_node(0).await?;
    ///
    /// // Node continues from where it left off
    /// fixture.l1().sync().await?;
    /// fixture.expect_event().l1_synced().await?;
    /// ```
    pub async fn start_node(&mut self, node_index: usize) -> eyre::Result<()> {
        if node_index >= self.nodes.len() {
            return Err(eyre::eyre!(
                "Node index {} out of bounds (total nodes: {})",
                node_index,
                self.nodes.len()
            ));
        }

        if self.nodes[node_index].is_some() {
            return Err(eyre::eyre!("Node at index {} is already running", node_index));
        }

        tracing::info!("Starting node at index {} (reusing database)", node_index);

        // Step 2: Create a new node instance with the existing database
        let (mut new_nodes, _, _) = setup_engine(
            &self.tasks,
            self.config.clone(),
            1,
            self.chain_spec.clone(),
            true,
            false,
            Some((node_index, self.dbs[node_index].clone())),
        )
        .await?;

        if new_nodes.is_empty() {
            return Err(eyre::eyre!("Failed to create new node"));
        }

        let new_node = new_nodes.remove(0);

        // Step 3: Setup Engine API client
        let auth_client = new_node.inner.engine_http_client();
        let engine_client = Arc::new(ScrollAuthApiEngineClient::new(auth_client))
            as Arc<dyn ScrollEngineApi + Send + Sync + 'static>;

        // Step 4: Get necessary handles from the new node
        let l1_watcher_handle = new_node.inner.add_ons_handle.l1_watcher_handle.clone();
        let rollup_manager_handle = new_node.inner.add_ons_handle.rollup_manager_handle.clone();

        // Step 5: Restore ForkchoiceState from persisted ChainOrchestrator status
        // CRITICAL: This must NOT be reset to genesis. The execution client's state
        // is at a later block, and resetting FCS to genesis would cause a mismatch,
        // resulting in "Syncing" errors when building payloads after reboot.
        let status = rollup_manager_handle.status().await?;
        let fcs: scroll_engine::ForkchoiceState = status.l2.fcs;

        tracing::info!(
            "Restored FCS from database - head: {:?}, safe: {:?}, finalized: {:?}",
            fcs.head_block_info(),
            fcs.safe_block_info(),
            fcs.finalized_block_info()
        );

        // Step 6: Initialize Engine with the restored ForkchoiceState
        let engine = Engine::new(Arc::new(engine_client), fcs);
        let chain_orchestrator_rx =
            new_node.inner.add_ons_handle.rollup_manager_handle.get_event_listener().await?;

        // Step 7: Determine node type (sequencer vs follower)
        let was_sequencer = self.config.sequencer_args.sequencer_enabled && node_index == 0;

        // Step 8: Create the new NodeHandle with all components
        let new_node_handle = NodeHandle {
            node: new_node,
            engine,
            chain_orchestrator_rx,
            l1_watcher_tx,
            rollup_manager_handle,
            typ: if was_sequencer {
                crate::test_utils::fixture::NodeType::Sequencer
            } else {
                crate::test_utils::fixture::NodeType::Follower
            },
        };

        // Step 9: Replace the old (None) node slot with the new node
        self.nodes[node_index] = Some(new_node_handle);
        tracing::info!("Node started successfully at index {}", node_index);

        Ok(())
    }
}
