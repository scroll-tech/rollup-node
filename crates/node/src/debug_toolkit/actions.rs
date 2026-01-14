//! Custom action framework for the debug toolkit.
//!
//! Users can implement the [`Action`] trait to create custom commands
//! that have full access to the [`TestFixture`].
//!
//! # Example
//!
//! ```rust,ignore
//! use rollup_node::debug_toolkit::actions::{Action, ActionRegistry};
//! use rollup_node::test_utils::TestFixture;
//! use async_trait::async_trait;
//!
//! struct MyAction;
//!
//! #[async_trait]
//! impl Action for MyAction {
//!     fn name(&self) -> &'static str {
//!         "my-action"
//!     }
//!
//!     fn description(&self) -> &'static str {
//!         "Does something cool with the fixture"
//!     }
//!
//!     async fn execute(
//!         &self,
//!         fixture: &mut TestFixture,
//!         args: &[String],
//!     ) -> eyre::Result<()> {
//!         // Your custom logic here
//!         println!("Running my action with {} nodes!", fixture.nodes.len());
//!         Ok(())
//!     }
//! }
//!
//! // Register in ActionRegistry::new()
//! ```

use crate::test_utils::TestFixture;
use async_trait::async_trait;
use colored::Colorize;
use futures::StreamExt;
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;

/// Trait for custom debug actions.
///
/// Implement this trait to create actions that can be invoked via `run <name>`.
#[async_trait]
pub trait Action: Send + Sync {
    /// Name of the action (used in `run <name>`).
    fn name(&self) -> &'static str;

    /// Short description shown in `run list`.
    fn description(&self) -> &'static str;

    /// Optional usage string for help.
    fn usage(&self) -> Option<&'static str> {
        None
    }

    /// Execute the action with full access to the fixture.
    ///
    /// # Arguments
    /// * `fixture` - Mutable reference to the test fixture
    /// * `args` - Arguments passed after the action name
    async fn execute(&self, fixture: &mut TestFixture, args: &[String]) -> eyre::Result<()>;
}

/// Registry of available actions.
pub struct ActionRegistry {
    actions: Vec<Box<dyn Action>>,
}

impl std::fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionRegistry").field("action_count", &self.actions.len()).finish()
    }
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionRegistry {
    /// Create a new registry with built-in actions.
    pub fn new() -> Self {
        let mut registry = Self { actions: Vec::new() };

        // Register built-in example actions
        registry.register(Box::new(BuildBlocksAction));
        registry.register(Box::new(StressTestAction));
        registry.register(Box::new(SyncAllAction));

        // ========================================
        // ADD YOUR CUSTOM ACTIONS HERE:
        // registry.register(Box::new(MyCustomAction));
        // ========================================

        registry
    }

    /// Register a new action.
    pub fn register(&mut self, action: Box<dyn Action>) {
        self.actions.push(action);
    }

    /// Get an action by name.
    pub fn get(&self, name: &str) -> Option<&dyn Action> {
        self.actions.iter().find(|a| a.name() == name).map(|a| a.as_ref())
    }

    /// List all registered actions.
    pub fn list(&self) -> impl Iterator<Item = &dyn Action> {
        self.actions.iter().map(|a| a.as_ref())
    }
}

// ============================================================================
// Built-in Example Actions
// ============================================================================

/// Build multiple blocks in sequence.
struct BuildBlocksAction;

#[async_trait]
impl Action for BuildBlocksAction {
    fn name(&self) -> &'static str {
        "build-blocks"
    }

    fn description(&self) -> &'static str {
        "Build multiple blocks in sequence"
    }

    fn usage(&self) -> Option<&'static str> {
        Some("run build-blocks <count> [timeout_ms]")
    }

    async fn execute(&self, fixture: &mut TestFixture, args: &[String]) -> eyre::Result<()> {
        let count: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(5);
        let timeout_ms: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5000);

        println!("Building {} blocks (timeout: {}ms per block)...", count, timeout_ms);

        let sequencer_idx = fixture
            .nodes
            .iter()
            .position(|n| n.is_sequencer())
            .ok_or_else(|| eyre::eyre!("No sequencer node found"))?;

        // Get an event listener for the sequencer
        let mut event_rx = fixture.nodes[sequencer_idx]
            .rollup_manager_handle
            .get_event_listener()
            .await
            .map_err(|e| eyre::eyre!("Failed to get event listener: {}", e))?;

        for i in 1..=count {
            fixture.nodes[sequencer_idx].rollup_manager_handle.build_block();
            print!("  Block {} triggered, waiting...", i);
            let _ = std::io::Write::flush(&mut std::io::stdout());

            // Wait for BlockSequenced event
            let timeout = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms));
            tokio::pin!(timeout);

            loop {
                tokio::select! {
                    event = event_rx.next() => {
                        if let Some(ChainOrchestratorEvent::BlockSequenced(block)) = event {
                            println!(" sequenced at #{}", block.header.number);
                            break;
                        }
                        // Continue waiting for BlockSequenced event
                    }
                    _ = &mut timeout => {
                        println!(" timeout!");
                        return Err(eyre::eyre!("Timeout waiting for block {} to be sequenced", i));
                    }
                }
            }
        }

        let status = fixture.nodes[sequencer_idx].rollup_manager_handle.status().await?;
        println!(
            "{}",
            format!("Done! Head is now at block #{}", status.l2.fcs.head_block_info().number)
                .green()
        );

        Ok(())
    }
}

/// Stress test by sending many transactions.
struct StressTestAction;

#[async_trait]
impl Action for StressTestAction {
    fn name(&self) -> &'static str {
        "stress-test"
    }

    fn description(&self) -> &'static str {
        "Send multiple transactions and build blocks"
    }

    fn usage(&self) -> Option<&'static str> {
        Some("run stress-test <tx_count> [build_every_n]")
    }

    async fn execute(&self, fixture: &mut TestFixture, args: &[String]) -> eyre::Result<()> {
        use alloy_consensus::{SignableTransaction, TxEip1559};
        use alloy_eips::eip2718::Encodable2718;
        use alloy_network::TxSignerSync;
        use alloy_primitives::{TxKind, U256};

        let tx_count: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);
        let build_every: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);

        println!("Stress test: {} txs, build every {} txs", tx_count, build_every);

        let sequencer_idx = fixture
            .nodes
            .iter()
            .position(|n| n.is_sequencer())
            .ok_or_else(|| eyre::eyre!("No sequencer node found"))?;

        let mut wallet = fixture.wallet.lock().await;
        let chain_id = wallet.chain_id;
        let signer = wallet.inner.clone();
        let to_address = signer.address(); // Send to self

        for i in 0..tx_count {
            let nonce = wallet.inner_nonce;
            wallet.inner_nonce += 1;

            let mut tx = TxEip1559 {
                chain_id,
                nonce,
                gas_limit: 21000,
                max_fee_per_gas: 1_000_000_000,
                max_priority_fee_per_gas: 1_000_000_000,
                to: TxKind::Call(to_address),
                value: U256::from(1),
                access_list: Default::default(),
                input: Default::default(),
            };

            let signature = signer.sign_transaction_sync(&mut tx)?;
            let signed = tx.into_signed(signature);
            let raw_tx = alloy_primitives::Bytes::from(signed.encoded_2718());

            // Need to drop wallet lock to inject
            drop(wallet);

            fixture.inject_tx_on(sequencer_idx, raw_tx).await?;
            print!(".");

            // Re-acquire wallet lock
            wallet = fixture.wallet.lock().await;

            // Build block periodically
            if (i + 1) % build_every == 0 {
                drop(wallet);
                fixture.nodes[sequencer_idx].rollup_manager_handle.build_block();
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                wallet = fixture.wallet.lock().await;
                print!("B");
            }
        }

        drop(wallet);

        // Final build
        fixture.nodes[sequencer_idx].rollup_manager_handle.build_block();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        println!();
        let status = fixture.nodes[sequencer_idx].rollup_manager_handle.status().await?;
        println!(
            "{}",
            format!(
                "Done! Sent {} txs, head at block #{}",
                tx_count,
                status.l2.fcs.head_block_info().number
            )
            .green()
        );

        Ok(())
    }
}

/// Ensure L1 is synced on all nodes.
struct SyncAllAction;

#[async_trait]
impl Action for SyncAllAction {
    fn name(&self) -> &'static str {
        "sync-all"
    }

    fn description(&self) -> &'static str {
        "Send L1 sync event to all nodes"
    }

    fn usage(&self) -> Option<&'static str> {
        None
    }

    async fn execute(&self, fixture: &mut TestFixture, _args: &[String]) -> eyre::Result<()> {
        println!("Syncing L1 on all {} nodes...", fixture.nodes.len());

        fixture.l1().sync().await?;

        println!("{}", "All nodes synced!".green());
        Ok(())
    }
}

// ============================================================================
// Template for custom actions - copy this to create your own!
// ============================================================================

#[allow(dead_code)]
struct TemplateAction;

#[allow(dead_code)]
#[async_trait]
impl Action for TemplateAction {
    fn name(&self) -> &'static str {
        "template"
    }

    fn description(&self) -> &'static str {
        "Template action - copy and modify!"
    }

    fn usage(&self) -> Option<&'static str> {
        Some("run template [arg1] [arg2]")
    }

    async fn execute(&self, fixture: &mut TestFixture, args: &[String]) -> eyre::Result<()> {
        // Access nodes
        println!("Fixture has {} nodes", fixture.nodes.len());

        // Access wallet
        let wallet = fixture.wallet.lock().await;
        println!("Wallet address: {:?}", wallet.inner.address());
        drop(wallet);

        // Access L1 provider
        // fixture.l1().sync().await?;

        // Access specific node
        let node = &fixture.nodes[0];
        let status = node.rollup_manager_handle.status().await?;
        println!("Head block: {}", status.l2.fcs.head_block_info().number);

        // Process arguments
        for (i, arg) in args.iter().enumerate() {
            println!("Arg {}: {}", i, arg);
        }

        Ok(())
    }
}
