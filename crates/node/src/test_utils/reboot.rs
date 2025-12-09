//! Reboot utilities for the rollup node.

use crate::test_utils::{setup_engine, NodeHandle, TestFixture};
use crate::ScrollRollupNodeConfig;
use rollup_node_primitives::BlockInfo;
use scroll_alloy_provider::{ScrollAuthApiEngineClient, ScrollEngineApi};
use scroll_engine::{Engine, ForkchoiceState};
use std::sync::Arc;

impl TestFixture {
    /// Shutdown a node by index, dropping its handle and cleaning up all resources.
    ///
    /// This will:
    /// - Remove the node from the fixture
    /// - Immediately drop the NodeHandle, triggering cleanup of:
    ///   - Network connections
    ///   - RPC server
    ///   - Database connections
    ///   - Background tasks
    ///   - All other resources
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
    /// // Now only 2 nodes remain
    /// assert_eq!(fixture.nodes.len(), 2);
    /// ```
    pub async fn shutdown_node(&mut self, node_index: usize) -> eyre::Result<()> {
        if node_index >= self.nodes.len() {
            return Err(eyre::eyre!("Node index {} out of bounds (total nodes: {})", node_index, self.nodes.len()));
        }

        tracing::info!("Shutting down node at index {}", node_index);

        // Remove and immediately drop the node handle
        let node_handle = self.nodes.remove(node_index);
        drop(node_handle);
        // All resources are cleaned up here

        Ok(())
    }

    /// Reboot a node by shutting it down and recreating it.
    ///
    /// **Note:** This is a complex operation with limitations:
    /// - The rebooted node will be a fresh instance
    /// - Database state will be lost (uses TmpDB)
    /// - Network connections need to be re-established
    /// - The node will start from genesis
    ///
    /// For most test scenarios, consider creating a new TestFixture instead.
    ///
    /// # Arguments
    /// * `node_index` - Index of the node to reboot
    /// * `config` - Configuration for the new node
    ///
    /// # Example
    /// ```ignore
    /// let mut fixture = TestFixture::builder().sequencer().build().await?;
    /// let config = default_test_scroll_rollup_node_config();
    ///
    /// // Reboot the sequencer
    /// fixture.reboot_node(0, config).await?;
    /// ```
    pub async fn reboot_node(
        &mut self,
        node_index: usize,
        config: ScrollRollupNodeConfig,
    ) -> eyre::Result<()> {
        if node_index >= self.nodes.len() {
            return Err(eyre::eyre!("Node index {} out of bounds (total nodes: {})", node_index, self.nodes.len()));
        }

        // Determine if this was a sequencer
        let was_sequencer = self.nodes[node_index].is_sequencer();
        tracing::info!(
            "Rebooting node at index {} (type: {})",
            node_index,
            if was_sequencer { "sequencer" } else { "follower" }
        );

        // Step 1: Shutdown the node (this drops it and cleans up resources)
        self.shutdown_node(node_index).await?;

        tracing::info!("Node shutdown complete, waiting for cleanup...");
        // Give some time for resources to be fully released
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Step 2: Create a new node
        tracing::info!("Creating new node...");
        let (mut new_nodes, _, _) = setup_engine(
            config.clone(),
            1, // Create just one node
            self.chain_spec.clone(),
            true, // is_dev
            false, // no_local_transactions_propagation
        ).await?;

        if new_nodes.is_empty() {
            return Err(eyre::eyre!("Failed to create new node"));
        }

        let new_node = new_nodes.remove(0);
        let genesis_hash = new_node.inner.chain_spec().genesis_hash();

        // Create engine for the new node
        let auth_client = new_node.inner.engine_http_client();
        let engine_client = Arc::new(ScrollAuthApiEngineClient::new(auth_client))
            as Arc<dyn ScrollEngineApi + Send + Sync + 'static>;
        let fcs = ForkchoiceState::new(
            BlockInfo { hash: genesis_hash, number: 0 },
            Default::default(),
            Default::default(),
        );
        let engine = Engine::new(Arc::new(engine_client), fcs);

        // Get handles
        let l1_watcher_tx = new_node.inner.add_ons_handle.l1_watcher_tx.clone();
        let rollup_manager_handle = new_node.inner.add_ons_handle.rollup_manager_handle.clone();
        let chain_orchestrator_rx = new_node.inner.add_ons_handle.rollup_manager_handle
            .get_event_listener()
            .await?;

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

        // Step 3: Insert the new node back at the same index
        self.nodes.insert(node_index, new_node_handle);
        tracing::info!("Node rebooted successfully at index {}", node_index);

        Ok(())
    }

    /// Shutdown all nodes in the fixture.
    ///
    /// This is equivalent to dropping the entire TestFixture, but allows
    /// you to do it explicitly while keeping the fixture structure intact.
    pub async fn shutdown_all(&mut self) -> eyre::Result<()> {
        tracing::info!("Shutting down all {} nodes", self.nodes.len());
        // Remove and drop all nodes
        while !self.nodes.is_empty() {
            let node = self.nodes.remove(0);
            drop(node);
        }
        tracing::info!("All nodes shut down");
        Ok(())
    }
}