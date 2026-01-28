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
