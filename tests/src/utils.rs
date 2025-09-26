use alloy_rpc_types_eth::BlockNumberOrTag;
use eyre::Result;
use std::time::Duration;

use crate::docker_compose::NamedProvider;

/// Enable automatic sequencing on a rollup node
pub async fn enable_automatic_sequencing(provider: &NamedProvider) -> Result<bool> {
    provider
        .client()
        .request("rollupNode_enableAutomaticSequencing", ())
        .await
        .map_err(|e| eyre::eyre!("Failed to enable automatic sequencing: {}", e))
}

/// Disable automatic sequencing on a rollup node
pub async fn disable_automatic_sequencing(provider: &NamedProvider) -> Result<bool> {
    provider
        .client()
        .request("rollupNode_disableAutomaticSequencing", ())
        .await
        .map_err(|e| eyre::eyre!("Failed to disable automatic sequencing: {}", e))
}

pub async fn miner_start(provider: &NamedProvider) -> Result<()> {
    provider
        .client()
        .request("miner_start", ())
        .await
        .map_err(|e| eyre::eyre!("Failed to start miner: {}", e))
}

pub async fn miner_stop(provider: &NamedProvider) -> Result<()> {
    provider
        .client()
        .request("miner_stop", ())
        .await
        .map_err(|e| eyre::eyre!("Failed to stop miner: {}", e))
}

/// Waits for all provided nodes to reach the target block number.
///
/// # Arguments
/// * `nodes` - Slice of NamedProvider structs containing provider and name
/// * `target_block` - The block number to wait for all nodes to reach
///
/// # Returns
/// * `Ok(())` if all nodes reach the target block within the timeout
/// * `Err` if timeout is reached or any provider call fails
pub async fn wait_for_block(nodes: &[&NamedProvider], target_block: u64) -> Result<()> {
    let timeout_duration = Duration::from_secs(60);
    let timeout_secs = timeout_duration.as_secs();

    tracing::info!(
        "⏳ Waiting for {} nodes to reach block {}... (timeout: {}s)",
        nodes.len(),
        target_block,
        timeout_secs
    );

    for i in 0..timeout_secs * 2 {
        let mut all_synced = true;
        let mut node_statuses = Vec::new();

        for node in nodes {
            let current_block = node.get_block_number().await?;
            node_statuses.push((node.name, current_block));

            if current_block < target_block {
                all_synced = false;
            }
        }

        if all_synced {
            tracing::info!("✅ All nodes reached target block {}", target_block);
            for (name, block) in node_statuses {
                tracing::info!("  - {}: block {}", name, block);
            }
            return Ok(());
        }

        // Log progress every 5 seconds
        if i % 10 == 0 {
            tracing::info!("Progress check ({}s elapsed):", i / 2);
            for (name, block) in node_statuses {
                tracing::info!(
                    "  - {}: block {} / {} {}",
                    name,
                    block,
                    target_block,
                    if block >= target_block { "✅" } else { "⏳" }
                );
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    eyre::bail!(
        "Timeout after {}s waiting for all nodes to reach block {}",
        timeout_secs,
        target_block
    )
}

/// Verifies that all provided nodes have the same block hash for a given block number.
///
/// # Arguments
/// * `nodes` - Slice of NamedProvider structs containing provider and name
/// * `block_number` - The block number to verify across all nodes
///
/// # Returns
/// * `Ok(())` if all nodes have the same block hash
/// * `Err` if any blocks are missing or hashes don't match
pub async fn assert_blocks_match(nodes: &[&NamedProvider], block_number: u64) -> Result<()> {
    if nodes.is_empty() {
        return Ok(());
    }

    let mut blocks = Vec::new();

    // Fetch blocks from all nodes
    for node in nodes {
        let block_opt = node.get_block_by_number(BlockNumberOrTag::Number(block_number)).await?;

        let block = block_opt
            .ok_or_else(|| eyre::eyre!("{} block {} not found", node.name, block_number))?;

        blocks.push((node.name, block));
    }

    // Get the reference hash from the first node
    let (ref_node_name, ref_block) = &blocks[0];
    let ref_hash = ref_block.header.hash;

    // Compare all other blocks to the reference
    for (node_name, block) in &blocks[1..] {
        let block_hash = block.header.hash;

        assert_eq!(
            block_hash, ref_hash,
            "Block {} hashes differ: {} has {:?}, {} has {:?}",
            block_number, ref_node_name, ref_hash, node_name, block_hash
        );
    }

    tracing::info!(
        "✅ Block {} matches across all {} nodes: hash={:?}",
        block_number,
        nodes.len(),
        ref_hash
    );

    Ok(())
}

pub async fn assert_latest_block(nodes: &[&NamedProvider], expected_block: u64) -> Result<u64> {
    if nodes.is_empty() {
        return Ok(0);
    }

    // Verify all nodes have the expected latest block
    for node in nodes {
        let block_number = node.get_block_number().await?;
        assert_eq!(
            block_number, expected_block,
            "{} is at block {}, expected {}",
            node.name, block_number, expected_block
        );
    }

    assert_blocks_match(nodes, expected_block).await?;

    tracing::info!(
        "✅ All {} nodes are at the expected latest block and hashes match",
        nodes.len()
    );

    Ok(expected_block)
}

/// Add a peer to the node's peer set via admin API
pub async fn admin_add_peer(provider: &NamedProvider, enode: &str) -> Result<bool> {
    provider
        .client()
        .request("admin_addPeer", (enode,))
        .await
        .map_err(|e| eyre::eyre!("Failed to add peer {}: {}", enode, e))
}

/// Remove a peer from the node's peer set via admin API
pub async fn admin_remove_peer(provider: &NamedProvider, enode: &str) -> Result<bool> {
    provider
        .client()
        .request("admin_removePeer", (enode,))
        .await
        .map_err(|e| eyre::eyre!("Failed to remove peer {}: {}", enode, e))
}

/// Add a trusted peer to the node's trusted peer set via admin API
pub async fn admin_add_trusted_peer(provider: &NamedProvider, enode: &str) -> Result<bool> {
    provider
        .client()
        .request("admin_addTrustedPeer", (enode,))
        .await
        .map_err(|e| eyre::eyre!("Failed to add trusted peer {}: {}", enode, e))
}

/// Remove a trusted peer from the node's trusted peer set via admin API
pub async fn admin_remove_trusted_peer(provider: &NamedProvider, enode: &str) -> Result<bool> {
    provider
        .client()
        .request("admin_removeTrustedPeer", (enode,))
        .await
        .map_err(|e| eyre::eyre!("Failed to remove trusted peer {}: {}", enode, e))
}
