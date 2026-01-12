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

use std::time::Duration;

use futures::StreamExt;

use crate::test_utils::{fixture::ScrollNodeTestComponents, setup_engine, NodeHandle, TestFixture};

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
        let NodeHandle { node, mut chain_orchestrator_rx, rollup_manager_handle: _r_h, typ: _ } =
            self.nodes
                .get_mut(node_index)
                .and_then(|opt| opt.take())
                .expect("Node existence checked above");
        let ScrollNodeTestComponents { node, task_manager, exit_future } = node;

        tokio::task::spawn_blocking(|| {
            if !task_manager.graceful_shutdown_with_timeout(Duration::from_secs(10)) {
                return Err(eyre::eyre!("Failed to shutdown tasks within timeout"));
            }
            eyre::Ok(())
        })
        .await??;

        // wait for the exit future to resolve - i.e. the consensus engine has shut down
        let _ = exit_future.await;

        // Wait for shutdown event from ChainOrchestrator
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = chain_orchestrator_rx.next().await {
                if matches!(event, rollup_node_chain_orchestrator::ChainOrchestratorEvent::Shutdown)
                {
                    return Ok::<(), eyre::ErrReport>(());
                }
            }
            Err(eyre::eyre!("Shutdown event not received"))
        })
        .await??;

        drop(node);

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
        let typ = if self.config.sequencer_args.sequencer_enabled && node_index == 0 {
            crate::test_utils::fixture::NodeType::Sequencer
        } else {
            crate::test_utils::fixture::NodeType::Follower
        };

        let new_node_handle = NodeHandle::new(new_node, typ).await?;
        self.nodes[node_index] = Some(new_node_handle);
        tracing::info!("Node started successfully at index {}", node_index);

        Ok(())
    }
}
