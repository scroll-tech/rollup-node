//! Tests for L1 event consumption in different sync states and L1 reorg handling.
//!
//! This test suite validates the rollup node's behavior when consuming batch events
//! from L1 (Ethereum) in various scenarios:
//!
//! # Test Categories
//!
//! ## 1. Sync State Behavior (Test Suite 1)
//! Tests how the node processes L1 events differently based on its sync state:
//! - **Pre-L1Synced**: Events are buffered during initial sync
//! - **Post-L1Synced**: Events are processed immediately as they arrive
//!
//! Covered events:
//! - `BatchCommit`: L2 blocks are committed to L1 (updates safe head)
//! - `BatchFinalized`: L2 blocks are finalized on L1 (updates finalized head)
//! - `BatchRevert`: L2 blocks are reverted on L1 (rolls back safe head)
//!
//! ## 2. L1 Reorg Handling (Test Suite 2)
//! Tests how the node handles L1 reorganizations that affect batch events:
//! - Reorging `BatchCommit`: Should revert safe head
//! - Reorging `BatchFinalized`: Should have no effect
//! - Reorging `BatchRevert`: Should restore safe head (undo the revert)
//!
//! # Architecture
//!
//! These tests use Anvil as a simulated L1 network, allowing precise control over:
//! - Transaction injection (batch events)
//! - Block production (mining)
//! - Chain reorganizations (`anvil_reorg` RPC)
//!
//! Related to: <https://github.com/scroll-tech/rollup-node/issues/420>

use alloy_primitives::Bytes;
use rollup_node::test_utils::{DatabaseOperations, EventAssertions, TestFixture};
use serde_json::Value;
use std::{collections::HashMap, sync::LazyLock};

/// Lazily load and cache all test transactions from `test_transactions.json`.
///
/// This loads the file only once and keeps all transactions in memory for efficient access
/// throughout the test suite. The cache is organized as a nested map:
/// `tx_type -> index -> raw_bytes`
static TEST_TRANSACTIONS: LazyLock<HashMap<String, HashMap<String, Bytes>>> = LazyLock::new(|| {
    let tx_json_path = "./tests/testdata/test_transactions.json";
    let tx_json_content = std::fs::read_to_string(tx_json_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", tx_json_path, e));

    let tx_data: Value = serde_json::from_str(&tx_json_content)
        .unwrap_or_else(|e| panic!("Failed to parse JSON: {}", e));

    let mut cache = HashMap::new();

    if let Value::Object(map) = tx_data {
        for (tx_type, tx_group) in map {
            if let Value::Object(transactions) = tx_group {
                let mut tx_map = HashMap::new();
                for (index, raw_tx_hex) in transactions {
                    if let Value::String(hex) = raw_tx_hex {
                        if !hex.is_empty() {
                            // Decode hex string to bytes
                            let raw_tx_bytes = if let Some(stripped) = hex.strip_prefix("0x") {
                                alloy_primitives::hex::decode(stripped)
                            } else {
                                alloy_primitives::hex::decode(hex.as_str())
                            }
                            .unwrap_or_else(|e| {
                                panic!("Failed to decode hex for {}.{}: {}", tx_type, index, e)
                            });
                            tx_map.insert(index, Bytes::from(raw_tx_bytes));
                        }
                    }
                }
                cache.insert(tx_type, tx_map);
            }
        }
    }

    cache
});

/// Helper to read pre-signed transactions from the cached transaction data.
///
/// The test data file contains signed transactions for various L1 batch operations:
/// - `commitBatch`: Transactions that commit L2 batches to the L1 contract
/// - `finalizeBatch`: Transactions that finalize previously committed batches
/// - `revertBatch`: Transactions that revert batches on L1
///
/// # Arguments
/// * `tx_type` - The transaction category (e.g., "commitBatch", "finalizeBatch")
/// * `index` - The transaction index within that category (e.g., "0", "1", "2")
///
/// # Returns
/// The raw transaction bytes ready to be sent to Anvil via `eth_sendRawTransaction`.
fn read_test_transaction(tx_type: &str, index: &str) -> eyre::Result<Bytes> {
    TEST_TRANSACTIONS
        .get(tx_type)
        .and_then(|tx_group| tx_group.get(index))
        .cloned()
        .ok_or_else(|| eyre::eyre!("Transaction not found: {}.{}", tx_type, index))
}

// =============================================================================
// Test Suite 1: Correct behavior when consuming events from L1
// =============================================================================

/// Test: `BatchCommit` events during initial syncing should be buffered and processed after
/// `L1Synced`.
///
/// # Test Flow
/// 1. Start a follower node in syncing state (skip automatic `L1Synced` notifications)
/// 2. Send multiple `BatchCommit` transactions to L1 (Anvil)
/// 3. Verify safe head remains at genesis (events are buffered, not processed)
/// 4. Trigger L1 sync completion by sending `L1Synced` notification
/// 5. Verify safe head advances after processing buffered `BatchCommit` events
///
/// # Expected Behavior
/// The node should not update the safe head during initial sync. Only after receiving
/// the `L1Synced` notification should it process the buffered `BatchCommit` events and
/// advance the safe head accordingly.
#[tokio::test]
async fn test_l1_sync_batch_commit() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup follower node in syncing state
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications() // Prevents automatic L1Synced, simulates initial sync
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    // Record initial state - should be at genesis
    let initial_status = fixture.get_status(0).await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;
    assert_eq!(initial_safe, 0, "Initial safe head should be at genesis (block 0)");

    // Step 2: Send BatchCommit transactions to L1 while node is syncing
    // These commits contain L2 blocks that should eventually become the safe head
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_commit_indexed().await?;
    }

    // Step 3: Verify safe head hasn't moved - events should be buffered
    let status = fixture.get_status(0).await?;
    assert_eq!(
        status.l2.fcs.safe_block_info().number,
        0,
        "Safe head should not change during syncing - BatchCommit events should be buffered"
    );

    // Step 4: Trigger L1 sync completion
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;
    for _ in 1..=6 {
        fixture.expect_event().batch_consolidated().await?;
    }

    // Step 5: Verify safe head advanced after processing buffered events
    let new_status = fixture.get_status(0).await?;
    assert!(
        new_status.l2.fcs.safe_block_info().number > initial_safe,
        "Safe head should advance after L1Synced when processing buffered BatchCommit events"
    );

    Ok(())
}

/// Test: `BatchFinalized` events are processed differently before and after `L1Synced`.
///
/// # Test Flow
/// 1. Start node in syncing state, send `BatchCommit` transactions (batches 0-6)
/// 2. Verify safe head remains at genesis during syncing
/// 3. Send `BatchFinalized` transactions (batches 1-3) while still syncing
/// 4. Verify both safe and finalized heads advance (special handling for finalized events)
/// 5. Trigger `L1Synced` to complete initial sync
/// 6. Send more `BatchFinalized` transactions (batches 4-6) after sync
/// 7. Verify only finalized head advances (safe head already set by commits)
///
/// # Expected Behavior
/// - Before `L1Synced`: `BatchFinalized` events update both safe and finalized heads because they
///   imply the batches are committed and finalized on L1.
/// - After `L1Synced`: `BatchFinalized` events only update the finalized head, as the safe head is
///   already managed by `BatchCommit` events.
#[tokio::test]
async fn test_l1_sync_batch_finalized() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup node in syncing state
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, Some(2))
        .build()
        .await?;

    // Record initial state
    let initial_status = fixture.get_status(0).await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;
    let initial_finalized = initial_status.l2.fcs.finalized_block_info().number;
    assert_eq!(
        (initial_safe, initial_finalized),
        (0, 0),
        "Initial safe and finalized heads should both be at genesis (block 0)"
    );

    // Step 2: Send BatchCommit transactions (batches 0-6) to L1
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_commit_indexed().await?;
    }

    // Step 3: Verify safe head hasn't changed (still syncing)
    let status = fixture.get_status(0).await?;
    assert_eq!(
        status.l2.fcs.safe_block_info().number,
        0,
        "Safe head should not change during syncing"
    );

    // Step 4: Send BatchFinalized transactions (batches 1-3) while syncing
    // Mine blocks to ensure the BatchFinalized events are themselves finalized on L1
    // This should trigger all unprocessed BatchCommit events up to the finalized batch
    for i in 1..=3 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_inject_tx(finalize_batch_tx).await?;
    }
    let anvil_block_number = fixture.anvil_get_block_number().await?;
    fixture.anvil_mine_blocks(4).await?;
    fixture.expect_event().l1_block_finalized_at_least(anvil_block_number).await?;

    // for i in 1..=3 {
    //     fixture.expect_event().batch_consolidated().await?;
    //     // Verify batch now has a finalized block number in database
    //     let finalized_block_number =
    //         fixture.db().get_batch_finalized_block_number_by_index(i).await?;
    //     assert!(
    //         matches!(finalized_block_number, Some(Some(n)) if n > 0),
    //         "Finalized block number should be greater than 0, got {:?}",
    //         finalized_block_number
    //     );
    // }

    // Step 5: Verify both safe and finalized heads advanced
    // During syncing, BatchFinalized implies the batch is both committed and finalized
    let batch_finalized_status = fixture.get_status(0).await?;
    assert!(
        batch_finalized_status.l2.fcs.safe_block_info().number > initial_safe,
        "Safe head should advance after BatchFinalized event during syncing"
    );
    assert!(
        batch_finalized_status.l2.fcs.finalized_block_info().number > initial_finalized,
        "Finalized head should advance after BatchFinalized event"
    );

    // Step 6: Complete L1 sync, this will process the buffered BatchCommit events
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;
    // for i in 4..=6 {
    //     fixture.expect_event().batch_consolidated().await?;
    //     let finalized_block_number =
    //         fixture.db().get_batch_finalized_block_number_by_index(i).await?;
    //     assert!(
    //         matches!(finalized_block_number, Some(None)),
    //         "Finalized block number should be None, got {:?}",
    //         finalized_block_number
    //     );
    // }
    let l1_synced_status = fixture.get_status(0).await?;
    assert!(
        l1_synced_status.l2.fcs.safe_block_info().number >
            batch_finalized_status.l2.fcs.safe_block_info().number,
        "Safe head should advance after L1 Synced when processing buffered BatchCommit events"
    );

    // Step 7: Send more BatchFinalized transactions (batches 4-6) after L1Synced
    for i in 4..=6 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_inject_tx(finalize_batch_tx).await?;
    }
    for i in 4..=6 {
        fixture.expect_event().batch_finalize_indexed().await?;
        let finalized_block_number =
            fixture.db().get_batch_finalized_block_number_by_index(i).await?;
        assert!(
            matches!(finalized_block_number, Some(Some(n)) if n > 0),
            "Finalized block number should be greater than 0, got {:?}",
            finalized_block_number
        );
    }
    let batch_finalized_status = fixture.get_status(0).await?;
    assert_eq!(
        batch_finalized_status.l2.fcs.finalized_block_info().number,
        l1_synced_status.l2.fcs.finalized_block_info().number,
        "Finalized head should not advance before BatchFinalized event are finalized on L1"
    );

    let anvil_block_number = fixture.anvil_get_block_number().await?;
    fixture.anvil_mine_blocks(4).await?;
    fixture.expect_event().l1_block_finalized_at_least(anvil_block_number).await?;

    // Step 8: Verify only finalized head advanced (safe head managed by BatchCommit)
    let batch_finalized_status = fixture.get_status(0).await?;
    assert!(
        batch_finalized_status.l2.fcs.safe_block_info().number ==
            l1_synced_status.l2.fcs.safe_block_info().number,
        "Safe head should not advance after BatchFinalized when L1 Synced (managed by BatchCommit)"
    );
    assert!(
        batch_finalized_status.l2.fcs.finalized_block_info().number >
            l1_synced_status.l2.fcs.finalized_block_info().number,
        "Finalized head should advance after BatchFinalized event when L1 Synced"
    );

    Ok(())
}

/// Test: `BatchRevert` events are ignored before `L1Synced`.
///
/// # Test Flow
/// 1. Start node in syncing state (skip automatic `L1Synced` notifications)
/// 2. Send `BatchCommit` transactions (batches 0-6) to L1
/// 3. Verify safe head remains at genesis (events are buffered during sync)
/// 4. Send a `BatchRevert` transaction to revert some batches
/// 5. Verify safe head still remains unchanged
///
/// # Expected Behavior
/// During initial sync (before `L1Synced`), `BatchRevert` events should be indexed but
/// NOT processed. The safe head should remain at genesis until L1 sync completes.
/// This prevents the node from processing revert events out of order during catchup.
#[tokio::test]
async fn test_l1_sync_batch_revert_before_l1_synced() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup node in syncing state
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    // Record initial state
    let initial_status = fixture.get_status(0).await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;
    assert_eq!(initial_safe, 0, "Initial safe head should be at genesis (block 0)");

    // Step 2: Send BatchCommit transactions (batches 0-6) to L1
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_commit_indexed().await?;
    }

    // Verify safe head advanced after processing commits
    let new_status = fixture.get_status(0).await?;
    assert_eq!(
        initial_safe,
        new_status.l2.fcs.safe_block_info().number,
        "Safe head should not advance after BatchCommit before L1Synced"
    );

    // Step 4: Send BatchRevert transaction to revert some batches
    let revert_batch_tx = read_test_transaction("revertBatch", "0")?;
    fixture.anvil_inject_tx(revert_batch_tx).await?;

    fixture.expect_event().batch_reverted().await?;

    // Step 5: Verify safe head decreased after revert
    let revert_status = fixture.get_status(0).await?;
    assert_eq!(
        revert_status.l2.fcs.safe_block_info().number,
        new_status.l2.fcs.safe_block_info().number,
        "Safe head should not change after BatchRevert before L1Synced"
    );

    Ok(())
}

/// Test: `BatchRevert` events correctly roll back the safe head after `L1Synced`.
///
/// # Test Flow
/// 1. Start node in syncing state
/// 2. Send `BatchCommit` transactions (batches 0-6) to L1 and wait for indexing
/// 3. Complete L1 sync by sending `L1Synced` notification
/// 4. Verify safe head advances after processing buffered commits
/// 5. Send a `BatchRevert` transaction to revert some batches
/// 6. Verify safe head decreases to reflect the reverted state
///
/// # Expected Behavior
/// After `L1Synced`, when a `BatchRevert` event is detected on L1, the node should
/// immediately roll back its safe head to the last valid batch before the reverted
/// batches. This ensures the L2 state remains consistent with the canonical L1 state
/// and handles L1 sequencer coordinator initiated reverts correctly.
#[tokio::test]
async fn test_l1_sync_batch_revert_after_l1_synced() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup node in syncing state
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    // Record initial state
    let initial_status = fixture.get_status(0).await?;
    let initial_safe = initial_status.l2.fcs.safe_block_info().number;
    assert_eq!(initial_safe, 0, "Initial safe head should be at genesis (block 0)");

    // Step 2: Send BatchCommit transactions (batches 0-6) to L1
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_commit_indexed().await?;
    }

    // Step 3: Complete L1 sync
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;
    for _ in 1..=6 {
        fixture.expect_event().batch_consolidated().await?;
    }

    // Verify safe head advanced after processing commits
    let new_status = fixture.get_status(0).await?;
    assert!(
        new_status.l2.fcs.safe_block_info().number > initial_safe,
        "Safe head should advance after BatchCommit when L1Synced"
    );

    // Step 4: Send BatchRevert transaction to revert some batches
    let revert_batch_tx = read_test_transaction("revertBatch", "0")?;
    fixture.anvil_inject_tx(revert_batch_tx).await?;

    fixture.expect_event().batch_reverted().await?;

    // Step 5: Verify safe head decreased after revert
    let revert_status = fixture.get_status(0).await?;
    assert!(
        revert_status.l2.fcs.safe_block_info().number < new_status.l2.fcs.safe_block_info().number,
        "Safe head should decrease after BatchRevert to reflect rolled-back state"
    );

    Ok(())
}

// =============================================================================
// Test Suite 2: L1 Reorg handling for different batch events
// =============================================================================

/// Test: L1 reorg removes `BatchCommit` events and correctly reverts the safe head.
///
/// # Test Flow
/// 1. Complete L1 sync first (node in `L1Synced` state)
/// 2. Send `BatchCommit` transactions (batches 0-3) and record safe head
/// 3. Send more `BatchCommit` transactions (batches 4-6) to advance safe head further
/// 4. Perform L1 reorg with depth 3 to remove batches 4-6
/// 5. Verify safe head reverted to the state after batch 3
///
/// # Expected Behavior
/// When an L1 reorg removes `BatchCommit` events, the node should:
/// - Detect the reorg via the L1 watcher
/// - Roll back the safe head to the last commit before the reorged blocks
/// - Maintain consistency between L1 and L2 state
///
/// This ensures that if the L1 sequencer coordinator reverts some batches due to
/// a reorganization, the L2 node follows suit and doesn't consider those blocks safe.
#[tokio::test]
async fn test_l1_reorg_batch_commit() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup and complete L1 sync
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, Some(32))
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send first batch of commits (batches 0-3)
    for i in 0..=3 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=3 {
        fixture.expect_event().batch_consolidated().await?;
    }

    // Record safe head after batch 3
    let status_after_batch_3 = fixture.get_status(0).await?;
    let safe_after_batch_3 = status_after_batch_3.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after batch 3: {}", safe_after_batch_3);

    // Step 3: Send more commits (batches 4-6) to advance safe head
    for i in 4..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=3 {
        fixture.expect_event().batch_consolidated().await?;
    }

    // Record advanced safe head after batch 6
    let status_after_batch_6 = fixture.get_status(0).await?;
    let safe_after_batch_6 = status_after_batch_6.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after batch 6: {}", safe_after_batch_6);
    assert!(
        safe_after_batch_6 > safe_after_batch_3,
        "Safe head should advance after additional BatchCommit events"
    );

    // Step 4: Perform L1 reorg to remove batches 4-6 (reorg depth 3)
    fixture.anvil_reorg(3).await?;

    // Wait for reorg detection
    fixture.expect_event().l1_reorg().await?;

    // Step 5: Verify safe head reverted to state after batch 3
    let status_after_reorg = fixture.get_status(0).await?;
    let safe_after_reorg = status_after_reorg.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after reorg: {}", safe_after_reorg);
    assert_eq!(
        safe_after_reorg, safe_after_batch_3,
        "Safe head should revert to last valid BatchCommit before reorg"
    );

    Ok(())
}

/// Test: L1 reorg of `BatchFinalized` events has no effect on the finalized head.
///
/// # Test Flow
/// 1. Send `BatchCommit` transactions (batches 0-6) to L1
/// 2. Send `BatchFinalized` transactions (batches 1-2) to L1
/// 3. Perform L1 reorg to remove the `BatchFinalized` events
/// 4. Verify finalized head remains unchanged despite the reorg
///
/// # Expected Behavior
/// The finalized head should NOT change when `BatchFinalized` events are reorged.
///
/// This is by design: The node only updates its finalized head when a `BatchFinalized`
/// event is **itself finalized on L1** (i.e., included in a finalized L1 block).
///
/// This test verifies the node handles such reorgs gracefully.
#[tokio::test]
async fn test_l1_reorg_batch_finalized() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup node
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send BatchCommit transactions (batches 0-6)
    // This commits the batches to L1 but doesn't finalize them yet
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for i in 1..=6 {
        fixture.expect_event().batch_consolidated().await?;
        // Verify batch exists in database but has no finalized block number yet
        let finalized_block_number =
            fixture.db().get_batch_finalized_block_number_by_index(i).await?;
        assert_eq!(
            finalized_block_number,
            Some(None),
            "Batch should exist but finalized block number should be None"
        );
    }

    // Step 3: Send BatchFinalized transactions (batches 1-2)
    // This finalizes the committed batches on L1
    for i in 1..=2 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_inject_tx(finalize_batch_tx).await?;
    }
    for i in 1..=2 {
        fixture.expect_event().batch_finalize_indexed().await?;
        // Verify batch now has a finalized block number in database
        let finalized_block_number =
            fixture.db().get_batch_finalized_block_number_by_index(i).await?;
        assert!(
            matches!(finalized_block_number, Some(Some(n)) if n > 0),
            "Finalized block number should be greater than 0, got {:?}",
            finalized_block_number
        );
    }

    // Record finalized head after finalization
    let status_after_finalize = fixture.get_status(0).await?;
    let finalized_after = status_after_finalize.l2.fcs.finalized_block_info().number;
    tracing::info!("Finalized head after batch finalized: {}", finalized_after);

    // Step 4: Perform L1 reorg to remove the BatchFinalized events (depth 2)
    fixture.anvil_reorg(2).await?;

    // Wait for reorg detection
    fixture.expect_event().l1_reorg().await?;

    // Step 5: Verify finalized head hasn't changed (reorg has no effect)
    let status_after_reorg = fixture.get_status(0).await?;
    let finalized_after_reorg = status_after_reorg.l2.fcs.finalized_block_info().number;
    tracing::info!("Finalized head after reorg: {}", finalized_after_reorg);
    assert_eq!(
        finalized_after_reorg, finalized_after,
        "Finalized head should not change after reorg of BatchFinalized (finality is irreversible)"
    );

    Ok(())
}

/// Test: L1 reorg that removes a `BatchRevert` event restores the safe head.
///
/// # Test Flow
/// 1. Complete L1 sync first
/// 2. Send `BatchCommit` transactions (batches 0-6) and record safe head
/// 3. Send a `BatchRevert` transaction that rolls back some batches
/// 4. Verify safe head decreased after the revert
/// 5. Perform L1 reorg to remove the `BatchRevert` event
/// 6. Verify safe head restored to pre-revert state
///
/// # Expected Behavior
/// When an L1 reorg removes a `BatchRevert` event, the node should:
/// - Detect the reorg via the L1 watcher
/// - Restore the safe head to the state before the (now non-existent) revert
/// - Treat the previously reverted batches as valid again
///
/// This scenario can occur if:
/// - A sequencer coordinator issues a revert transaction on L1
/// - That L1 block gets reorged before finalization
/// - The canonical L1 chain doesn't include the revert
///
/// The L2 node must track this and "undo the undo" - restoring the safe head
/// to include the batches that are no longer considered reverted.
#[tokio::test]
async fn test_l1_reorg_batch_revert() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup and complete L1 sync
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send BatchCommit transactions (batches 0-6)
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_consolidated().await?;
    }

    // Record safe head after all commits are processed
    let status_after_commits = fixture.get_status(0).await?;
    let safe_after_commits = status_after_commits.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after all commits: {}", safe_after_commits);

    // Step 3: Send BatchRevert transaction to roll back some batches
    let revert_batch_tx = read_test_transaction("revertBatch", "0")?;
    fixture.anvil_inject_tx(revert_batch_tx).await?;
    fixture.expect_event().batch_reverted().await?;

    // Step 4: Verify safe head decreased after revert
    let status_after_revert = fixture.get_status(0).await?;
    let safe_after_revert = status_after_revert.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after revert: {}", safe_after_revert);
    assert!(safe_after_revert < safe_after_commits, "Safe head should decrease after BatchRevert");

    // Step 5: Perform L1 reorg to remove the BatchRevert event (reorg depth 2)
    fixture.anvil_reorg(1).await?;
    fixture.anvil_mine_blocks(1).await?;
    fixture.expect_event().l1_reorg().await?;

    // Step 6: Verify safe head restored to pre-revert state
    // The batches are no longer reverted, so safe head should be back to full height
    let status_after_reorg = fixture.get_status(0).await?;
    let safe_after_reorg = status_after_reorg.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after reorg: {}", safe_after_reorg);
    assert_eq!(
        safe_after_reorg, safe_after_commits,
        "Safe head should be restored to pre-revert state after reorg removes BatchRevert"
    );

    Ok(())
}

/// Test: Node can correctly process `BatchCommit` events after reboot.
///
/// # Test Flow
/// 1. Setup and complete L1 sync.
/// 2. Send `BatchCommit` transactions (batches 0-3) and wait for consolidation.
/// 3. Verify safe head advanced.
/// 4. Shutdown the node.
/// 5. Send more `BatchCommit` transactions (batches 4-6) while node is down.
/// 6. Start the node and sync L1.
/// 7. Wait for batch consolidation events (batches 4-6).
/// 8. Verify safe head continues to advance correctly after reboot.
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_l1_sync_commit_batch_after_reboot() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup and complete L1 sync
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send BatchCommit transactions (batches 0-3)
    for i in 0..=3 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=3 {
        fixture.expect_event().batch_consolidated().await?;
    }

    let status_before_reboot = fixture.get_status(0).await?;
    let safe_before_reboot = status_before_reboot.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head before reboot: {}", safe_before_reboot);
    assert!(safe_before_reboot > 0, "Safe head should have advanced");

    // Step 3: Shutdown the node
    tracing::info!("Shutdowning node...");
    fixture.shutdown_node(0).await?;

    // Step 4: Send more BatchCommit transactions (batches 4-6)
    for i in 4..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }

    // Step 5: Start the node and sync
    fixture.start_node(0).await?;
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    for _ in 4..=6 {
        fixture.expect_event().batch_consolidated().await?;
    }

    // Step 6: Verify safe head continues to advance
    let status_after_reboot = fixture.get_status(0).await?;
    let safe_after_reboot = status_after_reboot.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after reboot: {}", safe_after_reboot);
    assert!(
        safe_after_reboot > safe_before_reboot,
        "Safe head should continue advancing after reboot"
    );

    Ok(())
}

/// Test: Node can correctly process `BatchFinalized` events after reboot.
///
/// # Test Flow
/// 1. Setup and complete L1 sync.
/// 2. Send `BatchCommit` transactions (batches 0-6) and wait for consolidation.
/// 3. Send `BatchFinalized` transactions (batches 1-3) and wait for indexing.
/// 4. Verify finalized head advanced.
/// 5. Shutdown the node.
/// 6. Send more `BatchFinalized` transactions (batches 4-6) while node is down.
/// 7. Start the node and sync L1.
/// 8. Wait for batch finalize indexed events (batches 4-6).
/// 9. Verify finalized head continues to advance correctly after reboot.
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_l1_sync_finalize_batch_after_reboot() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup and complete L1 sync
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send BatchCommit transactions (batches 0-6)
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_consolidated().await?;
    }

    let status_after_commit = fixture.get_status(0).await?;
    let safe_after_commit = status_after_commit.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after commits: {}", safe_after_commit);

    // Step 3: Send BatchFinalized transactions (batches 1-3)
    for i in 1..=3 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_inject_tx(finalize_batch_tx).await?;
    }
    for _ in 1..=3 {
        fixture.expect_event().batch_finalize_indexed().await?;
    }

    let status_before_reboot = fixture.get_status(0).await?;
    let finalized_before_reboot = status_before_reboot.l2.fcs.finalized_block_info().number;
    tracing::info!("Finalized head before reboot: {}", finalized_before_reboot);
    assert!(finalized_before_reboot > 0, "Finalized head should have advanced");

    // Step 4: Reboot the node
    tracing::info!("Rebooting node...");
    fixture.shutdown_node(0).await?;

    // Step 5: Send more BatchFinalized transactions (batches 4-6)
    for i in 4..=6 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_inject_tx(finalize_batch_tx).await?;
    }

    // Step 6: Start the node and sync
    fixture.start_node(0).await?;
    for _ in 4..=6 {
        fixture.expect_event().batch_finalize_indexed().await?;
    }
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 6: Verify finalized head continues to advance
    let status_after_reboot = fixture.get_status(0).await?;
    let finalized_after_reboot = status_after_reboot.l2.fcs.finalized_block_info().number;
    tracing::info!("Finalized head after reboot: {}", finalized_after_reboot);
    assert!(
        finalized_after_reboot > finalized_before_reboot,
        "Finalized head should continue advancing after reboot"
    );

    Ok(())
}

/// Test: Node can correctly process `BatchRevert` events after reboot.
///
/// # Test Flow
/// 1. Setup and complete L1 sync.
/// 2. Send `BatchCommit` transactions (batches 0-6) and wait for consolidation.
/// 3. Record safe head.
/// 4. Shutdown the node.
/// 5. Start the node and sync L1.
/// 6. Send `BatchRevert` transaction
/// 7. Wait for batch reverted event.
/// 8. Verify safe head decreased correctly.
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_l1_sync_revert_batch_after_reboot() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup and complete L1 sync
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send BatchCommit transactions (batches 0-6)
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_consolidated().await?;
    }

    let status_before_reboot = fixture.get_status(0).await?;
    let safe_before_reboot = status_before_reboot.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head before reboot: {}", safe_before_reboot);

    // Step 3: Shutdown the node
    tracing::info!("Rebooting node...");
    fixture.shutdown_node(0).await?;

    // Step 4: Start the node and sync
    fixture.start_node(0).await?;
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 5: Send BatchRevert transaction
    let revert_batch_tx = read_test_transaction("revertBatch", "0")?;
    fixture.anvil_inject_tx(revert_batch_tx).await?;
    fixture.expect_event().batch_reverted().await?;

    // Step 6: Verify safe head decreased
    let status_after_revert = fixture.get_status(0).await?;
    let safe_after_revert = status_after_revert.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after revert: {}", safe_after_revert);
    assert!(
        safe_after_revert < safe_before_reboot,
        "Safe head should decrease after BatchRevert following reboot"
    );

    Ok(())
}

/// Test: Node can correctly process `BatchCommit` events after reboot + L1 reorg.
///
/// # Test Flow
/// 1. Setup and complete L1 sync.
/// 2. Send `BatchCommit` transactions (batches 0-3) and wait for consolidation.
/// 3. Record safe head.
/// 4. Shutdown the node.
/// 5. Trigger L1 reorg (depth 1) to remove some `BatchCommit` events.
/// 6. Start the node and sync L1.
/// 7. Wait for L1 synced event.
/// 8. Verify safe head rolled back correctly after L1 reorg.
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_l1_reorg_commit_batch_after_reboot() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup and complete L1 sync
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send BatchCommit transactions (batches 0-3)
    for i in 0..=3 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=3 {
        fixture.expect_event().batch_consolidated().await?;
    }

    let status_before_reorg = fixture.get_status(0).await?;
    let safe_before_reorg = status_before_reorg.l2.fcs.safe_block_info().number;
    assert!(safe_before_reorg > 0, "Safe head should have advanced");
    tracing::info!("Safe head before reboot: {}", safe_before_reorg);

    // Step 3: Shutdown the node
    tracing::info!("Rebooting node...");
    fixture.shutdown_node(0).await?;

    // Step 4: Trigger L1 reorg (removes some BatchCommit events)
    fixture.anvil_reorg(1).await?;
    fixture.anvil_mine_blocks(1).await?;

    // Step 5: Start the node and sync
    fixture.start_node(0).await?;
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    let status_after_reorg = fixture.get_status(0).await?;
    let safe_after_reorg = status_after_reorg.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after reorg: {}", safe_after_reorg);

    // Step 6: Verify safe head advanced after new commits
    let status_final = fixture.get_status(0).await?;
    let safe_final = status_final.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after new commits: {}", safe_final);
    assert!(safe_before_reorg > safe_after_reorg, "Safe head should rollback after L1 reorg");

    Ok(())
}

/// Test: Node can correctly handle L1 reorg of `BatchFinalized` events after reboot.
///
/// # Test Flow
/// 1. Setup and complete L1 sync.
/// 2. Send `BatchCommit` transactions (batches 0-6) and wait for consolidation.
/// 3. Send `BatchFinalized` transactions (batches 1-3) and wait for indexing.
/// 4. Record finalized head.
/// 5. Shutdown the node.
/// 6. Trigger L1 reorg (depth 2) to remove `BatchFinalized` events.
/// 7. Expect L1 reorg event.
/// 8. Start the node and sync L1.
/// 9. Verify finalized head remains the same (reorg removed unfinalized L1 blocks).
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_l1_reorg_finalize_batch_after_reboot() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup and complete L1 sync
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, Some(32))
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send BatchCommit transactions (batches 0-6)
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_consolidated().await?;
    }

    // Step 3: Send BatchFinalized transactions (batches 1-3)
    for i in 1..=3 {
        let finalize_batch_tx = read_test_transaction("finalizeBatch", &i.to_string())?;
        fixture.anvil_inject_tx(finalize_batch_tx).await?;
    }
    for _ in 1..=3 {
        fixture.expect_event().batch_finalize_indexed().await?;
    }

    let status_before_reboot = fixture.get_status(0).await?;
    let finalized_before_reboot = status_before_reboot.l2.fcs.finalized_block_info().number;
    tracing::info!("Finalized head before reboot: {}", finalized_before_reboot);
    assert_eq!(finalized_before_reboot, 0, "Finalized head should be 0");

    // Step 4: Shutdown the node
    tracing::info!("Rebooting node...");
    fixture.shutdown_node(0).await?;

    // Step 5: Trigger L1 reorg (removes BatchFinalized events)
    fixture.anvil_reorg(2).await?;
    fixture.anvil_mine_blocks(2).await?;

    // Step 6: Start the node and sync
    fixture.start_node(0).await?;
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 6: Verify finalized head rolls back
    let status_after_reorg = fixture.get_status(0).await?;
    let finalized_after_reorg = status_after_reorg.l2.fcs.finalized_block_info().number;
    tracing::info!("Finalized head after reorg: {}", finalized_after_reorg);
    assert_eq!(
        finalized_after_reorg, finalized_before_reboot,
        "Finalized head should be the same after L1 reorg"
    );

    Ok(())
}

/// Test: Node can correctly handle L1 reorg of `BatchRevert` events after reboot.
///
/// # Test Flow
/// 1. Setup and complete L1 sync.
/// 2. Send `BatchCommit` transactions (batches 0-6) and wait for consolidation.
/// 3. Record safe head after commits.
/// 4. Send `BatchRevert` transaction and wait for batch reverted event.
/// 5. Verify safe head decreased after revert.
/// 6. Shutdown the node.
/// 7. Trigger L1 reorg (depth 1) to remove the `BatchRevert` event.
/// 8. Start the node and sync L1.
/// 9. Verify safe head is restored to pre-revert state (reorg undid the revert).
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_l1_reorg_revert_batch_after_reboot() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Step 1: Setup and complete L1 sync
    let mut fixture = TestFixture::builder()
        .followers(1)
        .skip_l1_synced_notifications()
        .with_anvil(None, None, Some(22222222), None, None)
        .build()
        .await?;

    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 2: Send BatchCommit transactions (batches 0-6)
    for i in 0..=6 {
        let commit_batch_tx = read_test_transaction("commitBatch", &i.to_string())?;
        fixture.anvil_inject_tx(commit_batch_tx).await?;
    }
    for _ in 1..=6 {
        fixture.expect_event().batch_consolidated().await?;
    }

    let status_after_commits = fixture.get_status(0).await?;
    let safe_after_commits = status_after_commits.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after commits: {}", safe_after_commits);

    // Step 3: Send BatchRevert transaction
    let revert_batch_tx = read_test_transaction("revertBatch", "0")?;
    fixture.anvil_inject_tx(revert_batch_tx).await?;
    fixture.expect_event().batch_reverted().await?;

    let status_after_revert = fixture.get_status(0).await?;
    let safe_after_revert = status_after_revert.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after revert: {}", safe_after_revert);
    assert!(safe_after_revert < safe_after_commits, "Safe head should decrease after BatchRevert");

    // Step 4: Shutdown the node
    tracing::info!("Rebooting node...");
    fixture.shutdown_node(0).await?;

    // Step 5: Trigger L1 reorg (removes BatchRevert event)
    fixture.anvil_reorg(1).await?;
    fixture.anvil_mine_blocks(1).await?;

    // Step 6: Start the node and sync
    fixture.start_node(0).await?;
    fixture.l1().sync().await?;
    fixture.expect_event().l1_synced().await?;

    // Step 7: Verify safe head is restored to pre-revert state
    let status_after_reorg = fixture.get_status(0).await?;
    let safe_after_reorg = status_after_reorg.l2.fcs.safe_block_info().number;
    tracing::info!("Safe head after reorg: {}", safe_after_reorg);
    assert_eq!(
        safe_after_reorg, safe_after_commits,
        "Safe head should be restored after L1 reorg removes BatchRevert event"
    );

    Ok(())
}
