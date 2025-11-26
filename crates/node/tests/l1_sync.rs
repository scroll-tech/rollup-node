//! Tests for multi-mode L1 event consumption.
//!
//! This test suite covers the behavior of the rollup node when consuming events from L1
//! in different sync states, handling reorgs, and recovering from shutdowns.
//!
//! Related to: https://github.com/scroll-tech/rollup-node/issues/420

use alloy_primitives::Bytes;
use reth_scroll_chainspec::SCROLL_DEV;
use rollup_node::test_utils::{EventAssertions, TestFixture};
use serde_json::Value;


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
async fn test_l1_sync_batch_commit() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder()
        .followers(1)
        .with_chain_spec(SCROLL_DEV.clone())
        .skip_l1_synced_notifications()
        .with_anvil_default_state()
        .with_anvil_chain_id(22222222)
        .build()
        .await?;

    // Get initial status
    let initial_status = fixture.get_sequencer_status().await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;

    // Send BatchCommit transactions
    for i in 0..=2 {
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
        .with_chain_spec(SCROLL_DEV.clone())
        .skip_l1_synced_notifications()
        .with_anvil_default_state()
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

    // Wait for l1 blocks to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check that finalized head was updated
    let batch_finalized_status = fixture.get_sequencer_status().await?;
    assert!(
        batch_finalized_status.l2.fcs.safe_block_info().number == l1_synced_status.l2.fcs.safe_block_info().number,
        "Safe head should not advance after BatchFinalized event when L1 Synced"
    );
    assert!(
        batch_finalized_status.l2.fcs.finalized_block_info().number > l1_synced_status.l2.fcs.finalized_block_info().number,
        "Finalized head should advance after BatchFinalized event when L1 Synced"
    );

    Ok(())
}

