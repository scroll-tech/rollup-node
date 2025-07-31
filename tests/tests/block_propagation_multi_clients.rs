//! Tests for block propagation to both geth and reth follower nodes.

use alloy_provider::Provider;
use alloy_rpc_types_eth::BlockNumberOrTag;
use eyre::Result;
use scroll_alloy_network::Scroll;
use std::time::Duration;
use tests::DockerComposeEnv;

#[tokio::test]
async fn test_docker_block_propagation_to_both_clients() -> Result<()> {
    tracing::info!("=== STARTING test_docker_block_propagation_to_both_clients ===");
    let env = DockerComposeEnv::new("multi-client-propagation").await?;

    let sequencer = env.get_sequencer_provider().await?;
    tracing::info!("✅ Sequencer provider created");

    let reth_follower = env.get_follower_provider().await?;
    tracing::info!("✅ Reth follower provider created");

    let geth_follower = env.get_l2geth_provider().await?;
    tracing::info!("✅ Geth follower provider created");

    // Verify all nodes have the same chain ID
    let seq_chain_id = sequencer.get_chain_id().await?;
    let reth_chain_id = reth_follower.get_chain_id().await?;
    let geth_chain_id = geth_follower.get_chain_id().await?;

    tracing::info!(
        "✅ All clients connected - Sequencer: {seq_chain_id}, Reth: {reth_chain_id}, Geth: {geth_chain_id}"
    );

    assert_eq!(seq_chain_id, reth_chain_id, "Sequencer and Reth chain IDs must match");
    assert_eq!(seq_chain_id, geth_chain_id, "Sequencer and Geth chain IDs must match");

    // Wait for sequencer to produce blocks
    let target_block = wait_for_sequencer_blocks(&sequencer, 15).await?;
    tracing::info!(
        "Sequencer produced {target_block} blocks, now waiting for followers to sync..."
    );

    // Wait for both followers to sync
    wait_for_follower_sync(&reth_follower, "Reth", target_block).await?;
    wait_for_follower_sync(&geth_follower, "Geth", target_block).await?;

    // Verify blocks match across all clients
    for block_num in 1..=target_block {
        verify_blocks_match_all_clients(&sequencer, &reth_follower, &geth_follower, block_num)
            .await?;
    }

    tracing::info!(
        "✅ Block hashes match across all clients for blocks 1-{target_block}. Multi-client propagation test completed successfully!"
    );

    Ok(())
}

/// Waits for the sequencer to produce a specific number of new blocks.
async fn wait_for_sequencer_blocks(
    sequencer: &impl Provider<Scroll>,
    num_blocks: u64,
) -> Result<u64> {
    let start_block = sequencer.get_block_number().await?;
    let target_block = start_block + num_blocks;
    tracing::info!(
        "⏳ Waiting for sequencer to produce {num_blocks} blocks (target: {target_block})..."
    );

    for _ in 0..15 {
        // 15 second timeout
        let current_block = sequencer.get_block_number().await?;
        if current_block >= target_block {
            tracing::info!("✅ Sequencer reached block {current_block}");
            return Ok(current_block);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    eyre::bail!("Timeout waiting for sequencer to produce blocks")
}

/// Waits for a follower to sync up to the target block.
async fn wait_for_follower_sync(
    follower: &impl Provider<Scroll>,
    client_name: &str,
    target_block: u64,
) -> Result<()> {
    tracing::info!("⏳ Waiting for {client_name} follower to sync to block {target_block}...");

    for _ in 0..20 {
        // 20 second timeout for followers
        let follower_block = follower.get_block_number().await?;
        if follower_block >= target_block {
            tracing::info!("✅ {client_name} follower synced to block {follower_block}");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    eyre::bail!("Timeout waiting for {client_name} follower to sync")
}

/// Verifies that block hashes match across all three clients for a given block number.
async fn verify_blocks_match_all_clients(
    sequencer: &impl Provider<Scroll>,
    reth_follower: &impl Provider<Scroll>,
    geth_follower: &impl Provider<Scroll>,
    block_number: u64,
) -> Result<()> {
    let seq_block_opt =
        sequencer.get_block_by_number(BlockNumberOrTag::Number(block_number)).await?;
    let reth_block_opt =
        reth_follower.get_block_by_number(BlockNumberOrTag::Number(block_number)).await?;
    let geth_block_opt =
        geth_follower.get_block_by_number(BlockNumberOrTag::Number(block_number)).await?;

    let seq_block =
        seq_block_opt.ok_or_else(|| eyre::eyre!("Sequencer block {} not found", block_number))?;
    let reth_block =
        reth_block_opt.ok_or_else(|| eyre::eyre!("Reth block {} not found", block_number))?;
    let geth_block =
        geth_block_opt.ok_or_else(|| eyre::eyre!("Geth block {} not found", block_number))?;

    // Compare block hashes
    let seq_hash = seq_block.header.hash;
    let reth_hash = reth_block.header.hash;
    let geth_hash = geth_block.header.hash;

    if seq_hash != reth_hash {
        eyre::bail!(
            "Block {} hashes differ between sequencer and reth: sequencer={:?}, reth={:?}",
            block_number,
            seq_hash,
            reth_hash
        );
    }

    if seq_hash != geth_hash {
        eyre::bail!(
            "Block {} hashes differ between sequencer and geth: sequencer={:?}, geth={:?}",
            block_number,
            seq_hash,
            geth_hash
        );
    }

    tracing::debug!("✅ Block {block_number} matches across all clients: hash={seq_hash:?}");
    Ok(())
}
