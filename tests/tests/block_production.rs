//! Tests for basic block production.

use alloy_provider::Provider;
use eyre::Result;
use scroll_alloy_network::Scroll;
use std::time::Duration;
use tests::DockerComposeEnv;

#[tokio::test]
async fn test_docker_block_production() -> Result<()> {
    println!("=== STARTING test_docker_block_production ===");
    let env = DockerComposeEnv::new("block-production").await?;

    let sequencer = env.get_sequencer_provider().await?;
    println!("✅ Sequencer provider created");

    let initial_block = sequencer.get_block_number().await?;
    println!("Initial block number: {initial_block}");

    let final_block = wait_for_sequencer_blocks(&sequencer, 20).await?;
    println!("Final block number: {final_block}");

    assert!(
        final_block >= initial_block + 5,
        "Sequencer should have produced at least 5 new blocks."
    );

    println!("✅ Block production test completed successfully!");
    Ok(())
}

/// Waits for the sequencer to produce a specific number of new blocks.
async fn wait_for_sequencer_blocks(
    sequencer: &impl Provider<Scroll>,
    num_blocks: u64,
) -> Result<u64> {
    let start_block = sequencer.get_block_number().await?;
    let target_block = start_block + num_blocks;
    println!("⏳ Waiting for sequencer to produce {num_blocks} blocks (target: {target_block})...");

    for _ in 0..10 {
        // 10 second timeout
        let current_block = sequencer.get_block_number().await?;
        if current_block >= target_block {
            println!("✅ Sequencer reached block {current_block}");
            return Ok(current_block);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    eyre::bail!("Timeout waiting for sequencer to produce blocks")
}
