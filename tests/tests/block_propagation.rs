//! Tests for basic block propagation.

mod common;
use common::retry_operation;

use alloy_network::Ethereum;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::{Block, BlockNumberOrTag};
use eyre::Result;
use std::time::Duration;
use tests::docker_compose::DockerComposeEnv;

#[tokio::test]
async fn test_block_propagation() -> Result<()> {
    println!("=== STARTING test_block_propagation ===");
    let _env = DockerComposeEnv::new("basic-block-propagation");

    // Wait for services to initialize.
    println!("⏳ Waiting for services to fully initialize...");
    _env.wait_for_services();

    // Create providers for both sequencer and follower.
    let sequencer_url = _env.get_sequencer_rpc_url();
    let follower_url = _env.get_follower_rpc_url();

    let sequencer =
        retry_operation(|| async { ProviderBuilder::new().connect(&sequencer_url).await }, 3)
            .await
            .expect("Failed to create sequencer provider");
    let follower =
        retry_operation(|| async { ProviderBuilder::new().connect(&follower_url).await }, 3)
            .await
            .expect("Failed to create follower provider");

    // Verify connectivity and matching chain IDs.
    let s_chain_id = sequencer.get_chain_id().await?;
    let f_chain_id = follower.get_chain_id().await?;
    println!(
        "✅ Sequencer (Chain ID: {s_chain_id}) & Follower (Chain ID: {f_chain_id}) connected."
    );
    assert_eq!(s_chain_id, f_chain_id, "Chain IDs must match");

    // 1. Wait for the sequencer to produce 5 new blocks.
    let target_block =
        wait_for_sequencer_blocks(&sequencer, 5).await.expect("Sequencer should produce blocks");

    // 2. Wait for the follower to sync up to the target block height.
    wait_for_follower_sync(&follower, target_block)
        .await
        .expect("Follower should sync to sequencer");

    // 3. Verify that the blocks on the follower match the blocks on the sequencer.
    for block_num in 1..=target_block {
        verify_blocks_match(&sequencer, &follower, block_num)
            .await
            .unwrap_or_else(|e| panic!("Block {block_num} mismatch: {e}"));
    }

    println!("✅ Basic block propagation test completed successfully!");
    Ok(())
}

/// Waits for the sequencer to produce a specific number of new blocks.
async fn wait_for_sequencer_blocks(
    sequencer: &impl Provider<Ethereum>,
    num_blocks: u64,
) -> Result<u64> {
    let start_block = sequencer.get_block_number().await?;
    let target_block = start_block + num_blocks;
    println!("⏳ Waiting for sequencer to produce {num_blocks} blocks (target: {target_block})...",);

    for _ in 0..60 {
        // 60 second timeout
        let current_block = sequencer.get_block_number().await?;
        if current_block >= target_block {
            println!("✅ Sequencer reached block {current_block}");
            return Ok(current_block);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    eyre::bail!("Timeout waiting for sequencer to produce blocks")
}

/// Waits for the follower to sync up to the target block.
async fn wait_for_follower_sync(
    follower: &impl Provider<Ethereum>,
    target_block: u64,
) -> Result<()> {
    println!("⏳ Waiting for follower to sync to block {target_block}...");

    for _ in 0..60 {
        // 60 second timeout
        let follower_block = follower.get_block_number().await?;
        if follower_block >= target_block {
            println!("✅ Follower synced to block {follower_block}");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    eyre::bail!("Timeout waiting for follower to sync")
}

/// Verifies that block hashes match between two nodes for a given block number.
async fn verify_blocks_match(
    sequencer: &impl Provider<Ethereum>,
    follower: &impl Provider<Ethereum>,
    block_number: u64,
) -> Result<()> {
    // CORRECTED: get_block_by_number now only takes one argument.
    let seq_block_opt: Option<Block> =
        sequencer.get_block_by_number(BlockNumberOrTag::Number(block_number)).await?;
    let fol_block_opt: Option<Block> =
        follower.get_block_by_number(BlockNumberOrTag::Number(block_number)).await?;

    let seq_block =
        seq_block_opt.ok_or_else(|| eyre::eyre!("Sequencer block {} not found", block_number))?;
    let fol_block =
        fol_block_opt.ok_or_else(|| eyre::eyre!("Follower block {} not found", block_number))?;

    // Compare block hashes.
    let seq_hash = seq_block.header.hash;
    let fol_hash = fol_block.header.hash;

    if seq_hash != fol_hash {
        eyre::bail!(
            "Block {} hashes differ: sequencer={:?}, follower={:?}",
            block_number,
            seq_hash,
            fol_hash
        );
    }

    println!("✅ Block {block_number} matches: hash={seq_hash:?}");
    Ok(())
}
