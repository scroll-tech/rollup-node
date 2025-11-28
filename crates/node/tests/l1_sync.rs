//! Tests for multi-mode L1 event consumption.
//!
//! This test suite covers the behavior of the rollup node when consuming events from L1
//! in different sync states, handling reorgs, and recovering from shutdowns.
//!
//! Related to: <https://github.com/scroll-tech/rollup-node/issues/420>

use alloy_primitives::Bytes;
use rollup_node::test_utils::{EventAssertions, TestFixture};
use serde_json::Value;

/// Helper to read transaction from `test_transactions.json`
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

/// Test: `BatchCommit` during Syncing state should have no effect.
///
/// Expected: The node should not update the safe head since we only process
/// `BatchCommit` eventds after the node is synced (post `L1Synced` notification).
#[tokio::test]
async fn test_l1_sync_batch_commit() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Get initial status
    let initial_status = fixture.get_sequencer_status().await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;

    // Send BatchCommit transactions
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(commit_batch_tx).await?;
    }

    // Wait for l1 blocks to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check status - safe head should still be at genesis
    let status = fixture.get_sequencer_status().await?;
    assert_eq!(
        status.l2.fcs.safe_block_info().number,
        0,
        "Safe head should not change during syncing"
    );

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;
    fixture.expect_event().batch_consolidated().await?;

    // Check that safe head was updated
    let new_status = fixture.get_sequencer_status().await?;
    assert!(
        new_status.l2.fcs.safe_block_info().number > initial_safe,
        "Safe head should advance after BatchCommit when synced and L1Synced"
    );

    Ok(())
}

#[tokio::test]
async fn test_l1_sync_batch_finalized() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Get initial status
    let initial_status = fixture.get_sequencer_status().await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;
    let initial_finalized = initial_status.l2.fcs.finalized_block_info().number;

    // Send BatchCommit transactions
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(commit_batch_tx).await?;
    }

    // Wait for l1 blocks to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check status - safe head should still be at genesis
    let status = fixture.get_sequencer_status().await?;
    assert_eq!(
        status.l2.fcs.safe_block_info().number,
        0,
        "Safe head should not change during syncing"
    );

    // Send BatchFinalized transactions
    for i in 1..=3 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(finalize_batch_tx).await?;
    }
    fixture.anvil_mine_blocks(64).await?;

    fixture.expect_event().batch_finalized().await?;

    // Wait for l1 blocks to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check that safe head was updated
    let batch_finalized_status = fixture.get_sequencer_status().await?;
    assert!(
        batch_finalized_status.l2.fcs.safe_block_info().number > initial_safe,
        "Safe head should advance after BatchFinalized event"
    );
    assert!(
        batch_finalized_status.l2.fcs.finalized_block_info().number > initial_finalized,
        "Finalized head should advance after BatchFinalizd event"
    );

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Wait for l1 blocks to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let l1_synced_status = fixture.get_sequencer_status().await?;

    // Send BatchFinalize transactions
    for i in 4..=6 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(finalize_batch_tx).await?;
    }
    fixture.anvil_mine_blocks(64).await?;

    // Wait for batch finalized event
    fixture.expect_event().batch_finalized().await?;

    // Check that finalized head was updated
    let batch_finalized_status = fixture.get_sequencer_status().await?;
    assert!(
        batch_finalized_status.l2.fcs.safe_block_info().number ==
            l1_synced_status.l2.fcs.safe_block_info().number,
        "Safe head should not advance after BatchFinalized event when L1 Synced"
    );
    assert!(
        batch_finalized_status.l2.fcs.finalized_block_info().number >
            l1_synced_status.l2.fcs.finalized_block_info().number,
        "Finalized head should advance after BatchFinalized event when L1 Synced"
    );

    Ok(())
}

#[tokio::test]
async fn test_l1_sync_batch_revert() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Get initial status
    let initial_status = fixture.get_sequencer_status().await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;

    // Send BatchCommit transactions
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(commit_batch_tx).await?;
    }

    fixture.anvil_mine_blocks(64).await?;

    // Trigger L1 synced event
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Wait for l1 blocks to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check that safe head was updated
    let new_status = fixture.get_sequencer_status().await?;
    assert!(
        new_status.l2.fcs.safe_block_info().number > initial_safe,
        "Safe head should advance after BatchCommit when L1Synced"
    );

    // Send BatchRevert transactions
    let revert_batch_tx = read_test_transaction("revertBatch", "0")?;
    fixture.anvil_send_raw_transaction(revert_batch_tx).await?;
    fixture.anvil_mine_blocks(10).await?;

    // Wait for l1 blocks to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check that safe head was updated
    let revert_status = fixture.get_sequencer_status().await?;
    assert!(
        revert_status.l2.fcs.safe_block_info().number < new_status.l2.fcs.safe_block_info().number,
        "Safe head should advance after BatchCommit when synced and L1Synced"
    );

    Ok(())
}

// =============================================================================
// Test Suite 2: L1 Reorg handling for different batch events
// =============================================================================

/// Test: L1 reorg of `BatchCommit` after `L1Synced`.
///
/// Expected: When a `BatchCommit` is reorged after the node has synced (`L1Synced`),
/// the safe head should revert to the last block of the previous `BatchCommit`.
#[tokio::test]
async fn test_l1_reorg_batch_commit() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Trigger L1 synced event
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Send BatchCommit transactions 0-3
    for i in 0..=3 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(commit_batch_tx).await?;
        if i != 0 {
            fixture.expect_event().batch_consolidated().await?;
        }
    }

    // Check that safe head was updated to batch 2
    let status_after_batch_3 = fixture.get_sequencer_status().await?;
    let safe_after_batch_3 = status_after_batch_3.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after batch 3: {}", safe_after_batch_3);

    // Send BatchCommit transactions 4-6
    for i in 4..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(commit_batch_tx).await?;
    }

    // Wait for processing
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check that safe head advanced to batch 3
    let status_after_batch_6: rollup_node_chain_orchestrator::ChainOrchestratorStatus =
        fixture.get_sequencer_status().await?;
    let safe_after_batch_6 = status_after_batch_6.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after batch 6: {}", safe_after_batch_6);
    assert!(
        safe_after_batch_6 > safe_after_batch_6,
        "Safe head should advance after BatchCommit when L1Synced"
    );

    // Reorg to remove batch 3 (reorg depth 1)
    fixture.anvil_reorg(3).await?;
    fixture.anvil_mine_blocks(1).await?;

    fixture.expect_event().l1_reorg().await?;

    // Check that safe head reverted
    let status_after_reorg = fixture.get_sequencer_status().await?;
    let safe_after_reorg = status_after_reorg.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after reorg: {}", safe_after_reorg);
    assert_eq!(
        safe_after_reorg, safe_after_batch_3,
        "Safe head should revert to previous BatchCommit after reorg"
    );

    Ok(())
}

/// Test: L1 reorg of `BatchFinalized` event.
///
/// Expected: Reorging `BatchFinalized` should have no effect because we only update
/// the finalized head after the `BatchFinalized` event is itself finalized on L1,
/// meaning it can never be reorged in practice.
#[tokio::test]
async fn test_l1_reorg_batch_finalized() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Send BatchCommit transactions
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(commit_batch_tx).await?;
    }

    // Send BatchFinalized transactions
    for i in 1..=2 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(finalize_batch_tx).await?;
    }

    // Trigger L1 sync
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Wait for consolidate and finalization
    fixture.expect_event().batch_consolidated().await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Get finalized head after batch finalization
    let status_after_finalize = fixture.get_sequencer_status().await?;
    let finalized_after = status_after_finalize.l2.fcs.finalized_block_info().number;
    tracing::info!("Finalized head after batch finalized: {}", finalized_after);

    // Reorg to remove the BatchFinalized events
    fixture.anvil_reorg(2).await?;
    fixture.anvil_mine_blocks(1).await?;

    // Wait for reorg detection
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check that finalized head hasn't changed
    let status_after_reorg = fixture.get_sequencer_status().await?;
    let finalized_after_reorg = status_after_reorg.l2.fcs.finalized_block_info().number;
    tracing::info!("Finalized head after reorg: {}", finalized_after_reorg);
    assert_eq!(
        finalized_after_reorg, finalized_after,
        "Finalized head should not change after reorg of BatchFinalized"
    );

    Ok(())
}

/// Test: L1 reorg of `BatchRevert` event.
///
/// Expected: When a `BatchRevert` is reorged, the safe head should be restored to
/// the state before the revert was applied. If the reverted batches are re-committed
/// after the reorg, the safe head should advance again.
#[tokio::test]
async fn test_l1_reorg_batch_revert() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Trigger L1 sync
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Send BatchCommit transactions
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_send_raw_transaction(commit_batch_tx).await?;
        // if i!=0 { fixture.expect_event().batch_consolidated().await?; }
    }

    // Wait for reorg to be detected and processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Get safe head after all commits
    let status_after_commits = fixture.get_sequencer_status().await?;
    let safe_after_commits = status_after_commits.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after all commits: {}", safe_after_commits);

    // Send BatchRevert transaction
    let revert_batch_tx = read_test_transaction("revertBatch", "0")?;
    fixture.anvil_send_raw_transaction(revert_batch_tx).await?;
    fixture.anvil_mine_blocks(1).await?;

    // Wait for revert to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check that safe head was reverted
    let status_after_revert = fixture.get_sequencer_status().await?;
    let safe_after_revert = status_after_revert.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after revert: {}", safe_after_revert);
    assert!(safe_after_revert < safe_after_commits, "Safe head should decrease after BatchRevert");

    // Reorg to remove the BatchRevert event (reorg depth 1)
    fixture.anvil_reorg(2).await?;
    fixture.anvil_mine_blocks(3).await?;

    // Wait for reorg to be detected and processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check that safe head was restored to pre-revert state
    let status_after_reorg = fixture.get_sequencer_status().await?;
    let safe_after_reorg = status_after_reorg.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after reorg: {}", safe_after_reorg);
    assert_eq!(
        safe_after_reorg, safe_after_commits,
        "Safe head should be restored after reorg of BatchRevert"
    );

    Ok(())
}
