use alloy_primitives::{hex::ToHexExt, Bytes};
use alloy_rpc_types_eth::BlockNumberOrTag;
use eyre::{Ok, Result};
use reth_e2e_test_utils::{transaction::TransactionTestContext, wallet::Wallet};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{sync::Mutex, time::interval};

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

pub fn create_wallet(chain_id: u64) -> Arc<Mutex<Wallet>> {
    Arc::new(Mutex::new(Wallet::default().with_chain_id(chain_id)))
}

/// Generate a transfer transaction with the given wallet.
pub async fn generate_tx(wallet: Arc<Mutex<Wallet>>) -> Bytes {
    let mut wallet = wallet.lock().await;
    let tx_fut = TransactionTestContext::transfer_tx_nonce_bytes(
        wallet.chain_id,
        wallet.inner.clone(),
        wallet.inner_nonce,
    );
    wallet.inner_nonce += 1;
    tx_fut.await
}

/// Send a raw transaction to multiple nodes, optionally waiting for confirmation.
pub async fn send_tx(
    wallet: Arc<Mutex<Wallet>>,
    nodes: &[&NamedProvider],
    wait_for_confirmation: bool,
) -> Result<()> {
    let tx = generate_tx(wallet).await;

    tracing::debug!("Sending transaction: {:?}", tx);
    let tx: Vec<u8> = tx.into();
    let mut pending_txs = Vec::new();

    for node in nodes {
        let builder = node.send_raw_transaction(&tx).await;
        match builder {
            std::result::Result::Ok(builder) => {
                let pending_tx = builder.register().await?;
                tracing::debug!(
                    "Sent transaction {:?} to node: {:?}",
                    pending_tx.tx_hash(),
                    node.name
                );
                pending_txs.push(pending_tx);
            }
            Err(e) => {
                if e.to_string().contains("already known") {
                    continue;
                }
                eyre::bail!("Failed to send transaction to node {}: {}", node.name, e);
            }
        };
    }

    if wait_for_confirmation {
        for pending_tx in pending_txs {
            let r = pending_tx.await?;
            tracing::debug!("Transaction confirmed: {:?}", r.encode_hex());
        }
    }

    Ok(())
}

/// Simple transaction sender that runs continuously until `stop` is set to true.
pub async fn run_continuous_tx_sender(stop: Arc<AtomicBool>, nodes: &[&NamedProvider]) -> u64 {
    let mut interval = interval(Duration::from_millis(50));
    let mut tx_count = 0u64;

    let wallet = create_wallet(nodes[0].get_chain_id().await.expect("Failed to get chain id"));

    while !stop.load(Ordering::Relaxed) {
        interval.tick().await;

        if let Err(e) = send_tx(wallet.clone(), nodes, false).await {
            tracing::error!("Error sending transaction: {}", e);
        } else {
            tx_count += 1;
        }
    }

    tx_count
}

pub async fn stop_continuous_tx_sender(
    stop: Arc<AtomicBool>,
    tx_sender: tokio::task::JoinHandle<u64>,
) -> Result<()> {
    stop.store(true, Ordering::Relaxed);
    let tx_count = tx_sender.await?;
    tracing::info!(
        "🔄 Stopped continuous transaction sender after sending {} transactions",
        tx_count
    );

    Ok(())
}
