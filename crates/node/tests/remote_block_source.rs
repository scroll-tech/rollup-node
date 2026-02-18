//! Integration tests for the `RemoteBlockSourceAddOn` feature.
//!
//! These tests verify that a node configured with `RemoteBlockSourceAddOn` can:
//! - Import blocks from a remote L2 node (the sequencer)
//! - Build new blocks on top of each imported block

use rollup_node::test_utils::{EventAssertions, TestFixture};

#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_remote_block_source() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder().sequencer().remote_source_node().build().await?;

    fixture.l1().sync().await?;

    // Sequencer produces blocks 1-5
    for i in 1..=5 {
        fixture.build_block().expect_block_number(i).build_and_await_block().await?;
        fixture.expect_event_on(1).block_sequenced(i + 1).await?;
    }

    Ok(())
}

/// Test that the remote block source correctly determines its resume point on restart.
///
/// The remote source's local chain has blocks 1-3 (imported from sequencer) plus
/// block 4 (built locally). The sequencer goes on to produce blocks 4-6. On restart,
/// the highest-common-block walk must identify block 3 (locally-built block 4 diverges
/// from sequencer's block 4) and import only blocks 4-6.
///
/// If the detection were broken (e.g. always returning 0), the remote source would try
/// to re-import blocks 1-6, producing 6 `BlockSequenced` events before reaching blocks
/// 5, 6, 7. This test asserts exactly three events in the correct order, confirming
/// the resume point is block 3.
#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn test_remote_block_source_resumes_from_correct_head() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut fixture = TestFixture::builder().sequencer().remote_source_node().build().await?;

    fixture.l1().sync().await?;

    // Sequencer produces blocks 1-3; remote source imports each and builds on top.
    // After this phase the remote source local chain is: 1, 2, 3 (sequencer) + 4 (local).
    for i in 1..=3u64 {
        fixture.build_block().expect_block_number(i).build_and_await_block().await?;
        fixture.expect_event_on(1).block_sequenced(i + 1).await?;
    }

    // Shut down the remote source node (index 1).
    fixture.shutdown_node(1).await?;

    // Sequencer produces blocks 4-6 while the remote source is offline.
    for i in 4..=6u64 {
        fixture.build_block().expect_block_number(i).build_and_await_block().await?;
    }

    // Restart the remote source.
    // Expected detection: local_head=4, remote_head=6, min=4.
    //   Block 4: local hash (locally built) ≠ remote hash (sequencer's) → walk back.
    //   Block 3: local hash == remote hash → last_imported_block = 3.
    // The add-on should therefore import blocks 4, 5, 6 and build 5, 6, 7 on top.
    fixture.start_node(1).await?;

    // Synchronise L1 state on the restarted remote source node.
    fixture.l1().for_node(1).sync().await?;

    // Verify the remote source catches up with the 3 missed sequencer blocks.
    fixture.expect_event_on(1).block_sequenced(5).await?;
    fixture.expect_event_on(1).block_sequenced(6).await?;
    fixture.expect_event_on(1).block_sequenced(7).await?;

    Ok(())
}
