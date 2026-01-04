//! Reboot utilities for rollup-node integration tests.
//!
//! Key behaviors:
//! - `shutdown_node()` asks the `ChainOrchestrator` to stop (best-effort), then drops the node
//!   handle to release RPC/network/DB resources. A short sleep is used to let async cleanup settle.
//! - `start_node()` reuses the existing DB and restores the engine `ForkchoiceState` from the
//!   rebooted node's status (avoids `Syncing` responses after restart).

use crate::test_utils::{setup_engine, NodeHandle, TestFixture};
use scroll_alloy_provider::{ScrollAuthApiEngineClient, ScrollEngineApi};
use scroll_engine::Engine;
use std::sync::Arc;

impl TestFixture {
    /// Shutdown a node by index.
    ///
    /// This will:
    /// - Set the node slot to `None` (preserving indices of other nodes)
    /// - Request the `ChainOrchestrator` to shutdown (best-effort)
    /// - Drop the `NodeHandle`, releasing RPC/network/DB handles and other resources
    /// - Wait briefly to reduce racey transient errors in tests
    ///
    /// The node can later be restarted using `start_node()`.
    ///
    /// # Arguments
    /// * `node_index` - Index of the node to shutdown (0 = sequencer if enabled)
    ///
    /// # Example
    /// ```ignore
    /// let mut fixture = TestFixture::builder().sequencer().followers(2).build().await?;
    ///
    /// // Shutdown follower node at index 1
    /// fixture.shutdown_node(1).await?;
    ///
    /// // Node indices remain the same, but node 1 is now None
    /// assert!(fixture.nodes[1].is_none());
    ///
    /// // Later, restart the node
    /// fixture.start_node(1, config).await?;
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

        // Step 1: Ask the ChainOrchestrator to stop (best-effort).
        if let Some(node) = &self.nodes[node_index] {
            tracing::info!("Sending shutdown command to ChainOrchestrator...");
            if let Err(e) = node.rollup_manager_handle.shutdown().await {
                tracing::warn!("Failed to send shutdown command to ChainOrchestrator: {}", e);
            } else {
                tracing::info!("ChainOrchestrator shutdown command acknowledged");
            }
        }

        // Step 2: Drop the node handle (releases RPC/network/DB handles, etc.).
        let node_handle = self.nodes[node_index].take();
        drop(node_handle);

        // Step 3: Brief wait to let async cleanup settle.
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        tracing::info!("Node at index {} shutdown complete", node_index);
        Ok(())
    }

    /// Start a previously shutdown node (reusing the existing database) at the given index.
    ///
    /// This allows you to restart a node that was shutdown, potentially with
    /// different configuration or to test node restart scenarios.
    ///
    /// # Arguments
    /// * `node_index` - Index of the node to start
    /// * `config` - Configuration for the node
    ///
    /// # Example
    /// ```ignore
    /// let mut fixture = TestFixture::builder().sequencer().build().await?;
    /// let config = default_test_scroll_rollup_node_config();
    ///
    /// // Shutdown the sequencer
    /// fixture.shutdown_node(0).await?;
    ///
    /// // Wait for some condition...
    /// tokio::time::sleep(Duration::from_secs(1)).await;
    ///
    /// // Restart the sequencer
    /// fixture.start_node(0, config).await?;
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

        // Create a new node, reusing the existing database for this index.
        let node_dbs = if node_index < self.dbs.len() {
            vec![self.dbs[node_index].clone()]
        } else {
            Vec::new() // Should not happen, but handle gracefully
        };

        let (mut new_nodes, _, _) = setup_engine(
            &self.tasks,
            self.config.clone(),
            1,        // Create just one node
            node_dbs, // Reuse the database
            self.chain_spec.clone(),
            true,  // is_dev
            false, // no_local_transactions_propagation
        )
        .await?;

        if new_nodes.is_empty() {
            return Err(eyre::eyre!("Failed to create new node"));
        }

        let new_node = new_nodes.remove(0);

        // Create engine client for the new node.
        let auth_client = new_node.inner.engine_http_client();
        let engine_client = Arc::new(ScrollAuthApiEngineClient::new(auth_client))
            as Arc<dyn ScrollEngineApi + Send + Sync + 'static>;

        // Get add-ons handles.
        let l1_watcher_tx = new_node.inner.add_ons_handle.l1_watcher_tx.clone();
        let rollup_manager_handle = new_node.inner.add_ons_handle.rollup_manager_handle.clone();

        // Restore ForkchoiceState from the rebooted node's status (do NOT reset to genesis).
        let status = rollup_manager_handle.status().await?;
        let fcs: scroll_engine::ForkchoiceState = status.l2.fcs;
        tracing::warn!(target: "scroll::test_utils::reboot", "fcs: {:?}", fcs);

        tracing::info!(
            "Restored FCS from database - head: {:?}, safe: {:?}, finalized: {:?}",
            fcs.head_block_info(),
            fcs.safe_block_info(),
            fcs.finalized_block_info()
        );

        let engine = Engine::new(Arc::new(engine_client), fcs);
        let chain_orchestrator_rx =
            new_node.inner.add_ons_handle.rollup_manager_handle.get_event_listener().await?;

        // Determine node type based on config and index
        let was_sequencer = self.config.sequencer_args.sequencer_enabled && node_index == 0;

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

        // Set the node at the index
        self.nodes[node_index] = Some(new_node_handle);
        tracing::info!("Node started successfully at index {}", node_index);

        Ok(())
    }
}
