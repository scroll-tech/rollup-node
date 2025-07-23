//! Tests for basic block propagation.

use alloy_network::Ethereum;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::{Block, BlockNumberOrTag};
use eyre::Result;
use std::time::Duration;
use tests::DockerComposeEnv;

#[tokio::test]
async fn test_block_propagation() -> Result<()> {
    println!("=== STARTING test_block_propagation ===");
    let env = DockerComposeEnv::new("basic-block-propagation");

    println!("⏳ Waiting for services to fully initialize...");
    DockerComposeEnv::wait_for_l2_node_ready(&env.get_sequencer_rpc_url(), 5).await?;
    DockerComposeEnv::wait_for_l2_node_ready(&env.get_follower_rpc_url(), 5).await?;

    let sequencer = ProviderBuilder::new().connect(&env.get_sequencer_rpc_url()).await?;
    println!("✅ Sequencer provider created");

    let follower = ProviderBuilder::new().connect(&env.get_follower_rpc_url()).await?;
    println!("✅ Follower provider created");

    let s_chain_id = sequencer.get_chain_id().await?;
    let f_chain_id = follower.get_chain_id().await?;
    println!(
        "✅ Sequencer (Chain ID: {s_chain_id}) & Follower (Chain ID: {f_chain_id}) connected."
    );
    assert_eq!(s_chain_id, f_chain_id, "Chain IDs must match");

    let target_block = wait_for_sequencer_blocks(&sequencer, 5).await?;
    println!("Sequencer produced {target_block} blocks, now waiting for follower sync...");

    wait_for_follower_sync(&follower, target_block).await?;
    println!("Follower synced to block {target_block}");

    for block_num in 1..=target_block {
        verify_blocks_match(&sequencer, &follower, block_num).await?;
    }
    println!("✅ Block hashes match for all blocks up to {target_block}");

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
