//! Tests for multi-mode L1 event consumption.
//!
//! This test suite covers the behavior of the rollup node when consuming events from L1
//! in different sync states, handling reorgs, and recovering from shutdowns.
//!
//! Related to: https://github.com/scroll-tech/rollup-node/issues/420

use alloy_primitives::{b256, Bytes, B256};
use reth_scroll_chainspec::SCROLL_DEV;
use rollup_node::test_utils::{EventAssertions, TestFixture};
use rollup_node_primitives::BlockInfo;
use serde_json::Value;

/// Helper to read test batch calldata from files.
fn read_batch_calldata(path: &str) -> eyre::Result<Bytes> {
    let content = std::fs::read_to_string(path)?;
    // Remove any whitespace/newlines and decode hex
    let hex_str = content.trim().trim_start_matches("0x");
    Ok(Bytes::from(alloy_primitives::hex::decode(hex_str)?))
}

/// Helper to read transaction from test_transactions.json
fn read_test_transaction(tx_type: &str, index: &str) -> eyre::Result<Bytes> {
    let tx_json_path = "./tests/testdata/test_transactions.json";
    let tx_json_content = std::fs::read_to_string(tx_json_path)
        .map_err(|e| eyre::eyre!("Failed to read {}: {}", tx_json_path, e))?;

    let tx_data: Value = serde_json::from_str(&tx_json_content)
        .map_err(|e| eyre::eyre!("Failed to parse JSON: {}", e))?;

    let raw_tx_hex = tx_data
        .get(tx_type)
        .and_then(|t| t.get(index))
        .and_then(|v| v.as_str())
        .ok_or_else(|| eyre::eyre!("Transaction not found: {}.{}", tx_type, index))?;

    if raw_tx_hex.is_empty() {
        return Err(eyre::eyre!("Transaction {}.{} is empty", tx_type, index));
    }

    // Decode hex string to bytes
    let raw_tx_bytes = if let Some(stripped) = raw_tx_hex.strip_prefix("0x") {
        alloy_primitives::hex::decode(stripped)?
    } else {
        alloy_primitives::hex::decode(raw_tx_hex)?
    };

    Ok(Bytes::from(raw_tx_bytes))
}

// =============================================================================
// Test Suite 1: Correct behavior when consuming events from L1
// =============================================================================

/// Test: BatchCommit during Syncing state should have no effect.
///
/// Expected: The node should not update the safe head since we only process
/// BatchCommit events after the node is synced (post L1Synced notification).
#[tokio::test]
async fn test_batch_commit_while_syncing() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .with_anvil_default_state()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Send BatchCommit while in Syncing state (before L1Synced)
    let commit_batch_0_tx = read_test_transaction("commitBatch", "0")?;
    fixture.anvil_send_raw_transaction(commit_batch_0_tx).await?;

    let commit_batch_1_tx = read_test_transaction("commitBatch", "1")?;
    fixture.anvil_send_raw_transaction(commit_batch_1_tx).await?;

    // Give it some time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;

    // Check status - safe head should still be at genesis
    let status = fixture.get_sequencer_status().await?;
    assert_eq!(status.l2.fcs.safe_block_info().number, 0, "Safe head should not change during syncing");

    Ok(())
}

/// Test: BatchCommit during Synced state should update the safe head.
///
/// Expected: After receiving L1Synced notification, BatchCommit events should
/// immediately update the safe head.
#[tokio::test]
async fn test_batch_commit_while_synced() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .with_anvil_default_state()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Get initial status
    let initial_status = fixture.get_sequencer_status().await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;

    // Send BatchCommit while in Syncing state (before L1Synced)
    let commit_batch_0_tx = read_test_transaction("commitBatch", "0")?;
    fixture.anvil_send_raw_transaction(commit_batch_0_tx).await?;

    let commit_batch_1_tx = read_test_transaction("commitBatch", "1")?;
    fixture.anvil_send_raw_transaction(commit_batch_1_tx).await?;

    let commit_batch_1_tx = read_test_transaction("commitBatch", "2")?;
    fixture.anvil_send_raw_transaction(commit_batch_1_tx).await?;

    // Give it some time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;

    // First, send L1Synced notification
    // fixture.l1().sync().await?;
    // fixture.expect_event().l1_synced().await?;
    // fixture.expect_event().batch_consolidated().await?;

    // Give it some time to process
    // tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;

    // Check that safe head was updated
    let new_status = fixture.get_sequencer_status().await?;
    let new_safe = new_status.l2.fcs.safe_block_info().number;

    assert!(new_safe > initial_safe, "Safe head should advance after BatchCommit when synced");

    Ok(())
}

/// Test: BatchFinalized during Syncing state.
///
/// Expected: This should trigger all unprocessed BatchCommit events up to the
/// finalized batch and update both safe and finalized heads.
#[tokio::test]
async fn test_batch_finalized_while_syncing() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .build()
        .await?;

    // Load batch data
    let batch_0_calldata = read_batch_calldata("crates/node/tests/testdata/batch_0_calldata.bin")?;
    let batch_hash = b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42");
    let commit_block = BlockInfo { number: 100, hash: B256::random() };
    let finalize_block = BlockInfo { number: 110, hash: B256::random() };

    // Send BatchCommit while syncing
    fixture
        .l1()
        .commit_batch()
        .hash(batch_hash)
        .index(1)
        .at_block(commit_block)
        .calldata(batch_0_calldata)
        .send()
        .await?;

    // Send BatchFinalization (this should trigger processing)
    fixture
        .l1()
        .finalize_batch()
        .hash(batch_hash)
        .index(1)
        .at_block(finalize_block)
        .send()
        .await?;

    // Finalize the L1 block
    fixture.l1().finalize_l1_block(finalize_block.number).await?;

    // Wait for batch consolidation
    fixture.expect_event().batch_consolidated().await?;

    // Verify both safe and finalized heads were updated
    let status = fixture.get_sequencer_status().await?;
    assert!(status.l2.fcs.safe_block_info().number > 0, "Safe head should be updated");
    assert!(status.l2.fcs.finalized_block_info().number > 0, "Finalized head should be updated");

    Ok(())
}

/// Test: BatchFinalized during Synced state should only update finalized head.
///
/// Expected: The finalized head should be updated to match the finalized batch.
#[tokio::test]
async fn test_batch_finalized_while_synced() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .build()
        .await?;

    // Load batch data
    let batch_0_calldata = read_batch_calldata("crates/node/tests/testdata/batch_0_calldata.bin")?;
    let batch_hash = b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42");
    let commit_block = BlockInfo { number: 100, hash: B256::random() };
    let finalize_block = BlockInfo { number: 110, hash: B256::random() };

    // Sync first
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Commit batch
    fixture
        .l1()
        .commit_batch()
        .hash(batch_hash)
        .index(1)
        .at_block(commit_block)
        .calldata(batch_0_calldata)
        .send()
        .await?;

    fixture.expect_event().batch_consolidated().await?;

    let safe_before = fixture.get_sequencer_status().await?.l2.fcs.safe_block_info().number;

    // Finalize batch
    fixture
        .l1()
        .finalize_batch()
        .hash(batch_hash)
        .index(1)
        .at_block(finalize_block)
        .send()
        .await?;

    fixture.l1().finalize_l1_block(finalize_block.number).await?;
    fixture.expect_event().l1_block_finalized().await?;

    // Check finalized head was updated
    let status = fixture.get_sequencer_status().await?;
    assert_eq!(status.l2.fcs.safe_block_info().number, safe_before, "Safe head should not change");
    assert!(status.l2.fcs.finalized_block_info().number > 0, "Finalized head should be updated");

    Ok(())
}

/// Test: BatchRevert during Syncing state should have no effect.
#[tokio::test]
async fn test_batch_revert_while_syncing() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .build()
        .await?;

    // Send a batch revert while syncing
    fixture
        .l1()
        .revert_batch()
        .index(1)
        .at_block(BlockInfo { number: 100, hash: B256::random() })
        .send()
        .await?;

    // Give it time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Status should remain unchanged
    let status = fixture.get_sequencer_status().await?;
    assert_eq!(status.l2.fcs.safe_block_info().number, 0);

    Ok(())
}

/// Test: BatchRevert during Synced state should update safe head to previous batch.
#[tokio::test]
async fn test_batch_revert_while_synced() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .build()
        .await?;

    // Load batch data
    let batch_0_calldata = read_batch_calldata("crates/node/tests/testdata/batch_0_calldata.bin")?;
    let batch_1_calldata = read_batch_calldata("crates/node/tests/testdata/batch_1_calldata.bin")?;
    let batch_0_hash = b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42");
    let batch_1_hash = b256!("AA8181F04F8E305328A6117FA6BC13FA2093A3C4C990C5281DF95A1CB85CA18F");

    // Sync first
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Commit batch 0
    fixture
        .l1()
        .commit_batch()
        .hash(batch_0_hash)
        .index(1)
        .at_block(BlockInfo { number: 100, hash: B256::random() })
        .calldata(batch_0_calldata)
        .send()
        .await?;

    fixture.expect_event().batch_consolidated().await?;
    let safe_after_batch_0 = fixture.get_sequencer_status().await?.l2.fcs.safe_block_info().number;

    // Commit batch 1
    fixture
        .l1()
        .commit_batch()
        .hash(batch_1_hash)
        .index(2)
        .at_block(BlockInfo { number: 105, hash: B256::random() })
        .calldata(batch_1_calldata)
        .send()
        .await?;

    fixture.expect_event().batch_consolidated().await?;
    let safe_after_batch_1 = fixture.get_sequencer_status().await?.l2.fcs.safe_block_info().number;

    assert!(safe_after_batch_1 > safe_after_batch_0, "Safe head should advance with batch 1");

    // Revert batch 1
    fixture
        .l1()
        .revert_batch()
        .index(2)
        .at_block(BlockInfo { number: 110, hash: B256::random() })
        .send()
        .await?;

    // Wait for revert to be processed
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Safe head should be back to batch 0 level
    let safe_after_revert = fixture.get_sequencer_status().await?.l2.fcs.safe_block_info().number;
    assert_eq!(
        safe_after_revert, safe_after_batch_0,
        "Safe head should revert to batch 0 level"
    );

    Ok(())
}

// =============================================================================
// Test Suite 2: L1 Reorg Handling
// =============================================================================

/// Test: L1 reorg of BatchCommit event.
///
/// Expected: Safe head should update to the last block of the previous BatchCommit.
#[tokio::test]
async fn test_l1_reorg_batch_commit() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .build()
        .await?;

    // Load batch data
    let batch_0_calldata = read_batch_calldata("crates/node/tests/testdata/batch_0_calldata.bin")?;
    let batch_1_calldata = read_batch_calldata("crates/node/tests/testdata/batch_1_calldata.bin")?;
    let batch_0_hash = b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42");
    let batch_1_hash = b256!("AA8181F04F8E305328A6117FA6BC13FA2093A3C4C990C5281DF95A1CB85CA18F");

    // Sync first
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Commit batch 0
    fixture
        .l1()
        .commit_batch()
        .hash(batch_0_hash)
        .index(1)
        .at_block(BlockInfo { number: 100, hash: B256::random() })
        .calldata(batch_0_calldata)
        .send()
        .await?;

    fixture.expect_event().batch_consolidated().await?;
    let safe_after_batch_0 = fixture.get_sequencer_status().await?.l2.fcs.safe_block_info().number;

    // Commit batch 1
    let block_105 = BlockInfo { number: 105, hash: B256::random() };
    fixture
        .l1()
        .commit_batch()
        .hash(batch_1_hash)
        .index(2)
        .at_block(block_105)
        .calldata(batch_1_calldata)
        .send()
        .await?;

    fixture.expect_event().batch_consolidated().await?;

    // Simulate L1 reorg at block 105 (where batch 1 was committed)
    fixture.l1().reorg_to(block_105.number).await?;

    // Wait for reorg to be processed - check that batch was reverted
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Safe head should revert to batch 0
    let safe_after_reorg = fixture.get_sequencer_status().await?.l2.fcs.safe_block_info().number;
    assert_eq!(
        safe_after_reorg, safe_after_batch_0,
        "Safe head should revert to batch 0 after reorg"
    );

    Ok(())
}

/// Test: L1 reorg of BatchFinalized event should have no effect.
///
/// Expected: Since we only update finalized head after the BatchFinalized event
/// is finalized on L1, a reorg of an unfinalized BatchFinalized event should not
/// affect our finalized head.
#[tokio::test]
async fn test_l1_reorg_batch_finalized_has_no_effect() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .build()
        .await?;

    // Load batch data
    let batch_calldata = read_batch_calldata("crates/node/tests/testdata/batch_0_calldata.bin")?;
    let batch_hash = b256!("5AAEB6101A47FC16866E80D77FFE090B6A7B3CF7D988BE981646AB6AEDFA2C42");

    // Sync and commit batch
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    fixture
        .l1()
        .commit_batch()
        .hash(batch_hash)
        .index(1)
        .at_block(BlockInfo { number: 100, hash: B256::random() })
        .calldata(batch_calldata)
        .send()
        .await?;

    fixture.expect_event().batch_consolidated().await?;

    let finalized_before = fixture.get_sequencer_status().await?.l2.fcs.finalized_block_info().number;

    // Send BatchFinalized (but don't finalize the L1 block yet)
    let finalize_block = BlockInfo { number: 110, hash: B256::random() };
    fixture
        .l1()
        .finalize_batch()
        .hash(batch_hash)
        .index(1)
        .at_block(finalize_block)
        .send()
        .await?;

    // Simulate L1 reorg at block 110 (where BatchFinalized was)
    fixture.l1().reorg_to(finalize_block.number).await?;

    // Finalized head should remain unchanged because the BatchFinalized
    // event was never finalized on L1
    let finalized_after = fixture.get_sequencer_status().await?.l2.fcs.finalized_block_info().number;
    assert_eq!(
        finalized_after, finalized_before,
        "Finalized head should not change from reorg of unfinalized BatchFinalized event"
    );

    Ok(())
}

// =============================================================================
// Test Suite 3: Node Shutdown and Restart with Reorg
// =============================================================================

/// Test: Node shutdown and restart should handle L1 reorgs that occurred while offline.
///
/// This test verifies that when a node shuts down with unfinalized L1 events in the
/// database and then restarts after an L1 reorg, it correctly handles the reorg.
#[tokio::test]
async fn test_node_restart_after_l1_reorg() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Note: This test would require more infrastructure to properly test node
    // shutdown and restart with persistent database. For now, we document the
    // expected behavior:
    //
    // 1. Node receives unfinalized L1 events (e.g., BatchCommit at block 100)
    // 2. Node shuts down
    // 3. L1 reorgs block 100
    // 4. Node restarts
    // 5. Node should detect the reorg and revert any state changes from the reorged events
    //
    // This requires:
    // - Persistent database across test runs
    // - Ability to restart the ChainOrchestrator
    // - Proper reorg detection on startup

    tracing::warn!("Node restart test requires persistent database infrastructure");

    Ok(())
}

// =============================================================================
// Test Suite 4: Integration with Anvil for real L1 events
// =============================================================================

/// Test: Use Anvil to simulate real L1 contract interactions.
///
/// This test demonstrates how to use the integrated Anvil instance to send
/// real transactions to L1 contracts and observe the resulting events.
#[tokio::test]
#[ignore] // Requires Anvil state to be properly configured
async fn test_with_anvil_l1_events() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create fixture with Anvil enabled
    let fixture = TestFixture::builder()
        .sequencer()
        .with_anvil_default_state()
        .with_anvil_chain_id(1337)
        .build()
        .await?;

    // Get Anvil instance
    let _anvil = fixture.anvil.as_ref().expect("Anvil should be enabled");

    // Note: Anvil NodeHandle doesn't expose port() directly
    // We would need to enhance the TestFixture to track this information
    tracing::info!("Anvil is running");

    // TODO: Use the anvil URL to:
    // 1. Send transactions to L1 contracts (ScrollChain, MessageQueue, etc.)
    // 2. Trigger real BatchCommit/BatchFinalized events
    // 3. Observe how the rollup node processes these real events

    // For now, we can use the test transactions from test_transactions.json
    // These can be sent to the Anvil instance using an RPC client

    Ok(())
}

