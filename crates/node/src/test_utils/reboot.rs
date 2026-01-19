//! Node reboot utilities for testing state persistence and recovery.
//!
//! Provides `shutdown_node()` and `start_node()` to test:
//! - State persistence across reboots
//! - ForkchoiceState restoration from database
//! - L1 event processing continuity

use std::time::Duration;

use futures::StreamExt;

use crate::test_utils::{fixture::ScrollNodeTestComponents, setup_engine, NodeHandle, TestFixture};

impl TestFixture {
    /// Gracefully shutdown a node and clean up its resources.
    /// 
    /// Process: Shutdown ChainOrchestrator → Drop NodeHandle → Wait for cleanup (1s).
    /// The node can be restarted later with `start_node()` to reuse its database.
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

        // Wait for consensus engine shutdown
        let _ = exit_future.await;

        // Wait for ChainOrchestrator shutdown event
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

        // Wait for async cleanup
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        tracing::info!("Node at index {} shutdown complete", node_index);
        Ok(())
    }

    /// Restart a previously shutdown node.
    ///
    /// Reuses the existing database and restores ForkchoiceState from persisted data.
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

        // Create node instance with existing database
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
