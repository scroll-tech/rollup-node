//! Tests for basic block production.

mod common;
use common::retry_operation;

use alloy_network::Ethereum;
use alloy_provider::{Provider, ProviderBuilder};
use eyre::Result;
use std::time::Duration;
use tests::docker_compose::DockerComposeEnv;

#[tokio::test]
async fn test_block_production() -> Result<()> {
    println!("=== STARTING test_block_production ===");
    let _env = DockerComposeEnv::new("block-production");

    // Wait for services to initialize.
    println!("⏳ Waiting 10 seconds for services to fully initialize...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Create a provider for the sequencer.
    let sequencer_url = _env.get_sequencer_rpc_url();
    let sequencer =
        match retry_operation(|| async { ProviderBuilder::new().connect(&sequencer_url).await }, 3)
            .await
        {
            Ok(provider) => {
                println!("✅ Sequencer provider created");
                provider
            }
            Err(e) => {
                _env.show_container_logs("rollup-node-sequencer");
                panic!("Failed to create sequencer provider: {e}");
            }
        };

    // Verify sequencer connectivity.
    match sequencer.get_chain_id().await {
        Ok(id) => println!("✅ Sequencer connected - Chain ID: {id}"),
        Err(e) => {
            _env.show_container_logs("rollup-node-sequencer");
            panic!("Failed to get chain ID from sequencer: {e}");
        }
    }

    // Wait for new blocks to be produced.
    let initial_block = retry_operation(|| async { sequencer.get_block_number().await }, 3).await?;
    println!("Initial block number: {initial_block}");

    let final_block = match wait_for_l2_blocks(&sequencer, 5, 60).await {
        Ok(num) => num,
        Err(e) => {
            _env.show_container_logs("rollup-node-sequencer");
            panic!("Failed while waiting for L2 blocks: {e}");
        }
    };
    println!("Final block number: {final_block}");

    assert!(
        final_block >= initial_block + 5,
        "Sequencer should have produced at least 5 new blocks."
    );

    println!("✅ Block production test completed successfully!");
    Ok(())
}

/// Waits for the sequencer to produce a specific number of new blocks within a timeout.
async fn wait_for_l2_blocks(
    sequencer: &impl Provider<Ethereum>,
    num_blocks: u64,
    timeout_secs: u64,
) -> Result<u64> {
    let start_block = sequencer.get_block_number().await?;
    let target_block = start_block + num_blocks;
    println!("⏳ Waiting for sequencer to produce {num_blocks} blocks (target: {target_block})...",);

    for i in 0..timeout_secs {
        let current_block = sequencer.get_block_number().await?;
        if current_block >= target_block {
            println!("✅ Sequencer reached block {current_block}");
            return Ok(current_block);
        }
        if i.is_multiple_of(5) {
            println!("⏳ Sequencer at block {current_block}/{target_block}");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    eyre::bail!("Timeout waiting for L2 blocks")
}
