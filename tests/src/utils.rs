use alloy_network::TransactionBuilder;
use alloy_primitives::{address, hex::ToHexExt, Address, Bytes, U256};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::{BlockId, BlockNumberOrTag, TransactionRequest};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{sol, SolCall};
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

// ===== L1 CONTRACT CONSTANTS =====

/// L1 node RPC URL for docker tests (port 8544 on host maps to 8545 in container)
const L1_RPC_URL: &str = "http://localhost:8544";

/// L1 deployer private key (first Anvil account)
const L1_DEPLOYER_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// L1 Scroll Messenger proxy address
const L1_SCROLL_MESSENGER_PROXY_ADDR: Address =
    address!("8A791620dd6260079BF849Dc5567aDC3F2FdC318");

/// L1 Enforced Transaction Gateway proxy address
const L1_ENFORCED_TX_GATEWAY_PROXY_ADDR: Address =
    address!("68B1D87F95878fE05B998F19b66F4baba5De1aed");

/// L1 Message Queue V2 proxy address
const L1_MESSAGE_QUEUE_V2_PROXY_ADDR: Address =
    address!("Dc64a140Aa3E981100a9becA4E685f962f0cF6C9");

// ===== L1 CONTRACT INTERFACES =====

sol! {
    /// L1 Scroll Messenger sendMessage function
    function sendMessage(
        address to,
        uint256 value,
        bytes memory message,
        uint256 gasLimit
    ) external payable;

    /// L1 Enforced Transaction Gateway sendTransaction function
    function sendTransaction(
        address target,
        uint256 value,
        uint256 gasLimit,
        bytes calldata data
    ) external payable;

    /// L1 Message Queue V2 nextCrossDomainMessageIndex function
    function nextCrossDomainMessageIndex() external view returns (uint256);
}

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

/// Get the latest relayed queue index from an l2geth node.
///
/// # Arguments
/// * `provider` - The L2 node provider (only l2geth)
///
/// # Returns
/// * `Ok(u64)` - The latest relayed queue index
/// * `Err` - If the RPC call fails or the value cannot be parsed
pub async fn scroll_get_latest_relayed_queue_index(provider: &NamedProvider) -> Result<u64> {
    let result: u64 = provider
        .client()
        .request("scroll_getLatestRelayedQueueIndex", ())
        .await
        .map_err(|e| eyre::eyre!("Failed to get latest relayed queue index: {}", e))?;

    Ok(result)
}

pub async fn wait_for_l1_message_queue_index_reached(
    nodes: &[&NamedProvider],
    expected_index: u64,
) -> Result<()> {
    let timeout_duration = Duration::from_secs(60);
    let timeout_secs = timeout_duration.as_secs();

    tracing::info!(
        "⏳ Waiting for {} nodes to reach queue index {}... (timeout: {}s)",
        nodes.len(),
        expected_index,
        timeout_secs
    );
    for i in 0..timeout_secs * 2 {
        let mut all_matched = true;
        let mut node_statuses = Vec::new();

        for node in nodes {
            let current_index = scroll_get_latest_relayed_queue_index(node).await?;
            node_statuses.push((node.name, current_index));

            if current_index < expected_index {
                all_matched = false;
            }
        }

        if all_matched {
            tracing::info!("✅ All nodes reached expected queue index {}", expected_index);
            for (name, index) in node_statuses {
                tracing::info!("  - {}: queue index {}", name, index);
            }
            return Ok(());
        }

        // Log progress every 5 seconds
        if i % 10 == 0 {
            tracing::info!("Progress check ({}s elapsed):", i / 2);
            for (name, index) in node_statuses {
                tracing::info!(
                    "  - {}: queue index {} / {} {}",
                    name,
                    index,
                    expected_index,
                    if index >= expected_index { "✅" } else { "⏳" }
                );
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Ok(())
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

/// Simple L1 message sender that runs continuously until `stop` is set to true.
pub async fn run_continuous_l1_message_sender(stop: Arc<AtomicBool>) -> () {
    while !stop.load(Ordering::Relaxed) {
        if let Err(e) = send_l1_scroll_messenger_message(
            address!("0000000000000000000000000000000000000001"),
            U256::from(1),
            Bytes::new(),
            200000,
            true,
        )
        .await
        {
            tracing::error!("Error sending L1 Scroll Messenger message: {}", e);
        }

        if let Err(e) = send_l1_enforced_tx_gateway_transaction(
            address!("0000000000000000000000000000000000000001"),
            U256::from(1),
            200000,
            Bytes::new(),
            true,
        )
        .await
        {
            tracing::error!("Error sending L1 Enforced Tx Gateway transaction: {}", e);
        }
    }
}

pub async fn stop_continuous_l1_message_sender(
    stop: Arc<AtomicBool>,
    l1_message_sender: tokio::task::JoinHandle<()>,
) -> Result<()> {
    stop.store(true, Ordering::Relaxed);
    l1_message_sender.await?;
    tracing::info!("🔄 Stopped continuous L1 message sender");

    Ok(())
}

/// Send a message via the L1 Scroll Messenger contract.
///
/// # Arguments
/// * `to` - The target address on L2
/// * `value` - The amount of wei to send with the message on L2
/// * `message` - The calldata to execute on L2
/// * `gas_limit` - The gas limit for executing the message on L2
pub async fn send_l1_scroll_messenger_message(
    to: Address,
    value: U256,
    message: Bytes,
    gas_limit: u64,
    wait_for_confirmation: bool,
) -> Result<()> {
    // Parse the private key and create a signer
    let signer: PrivateKeySigner = L1_DEPLOYER_PRIVATE_KEY
        .parse()
        .map_err(|e| eyre::eyre!("Failed to parse L1 deployer private key: {}", e))?;

    // Create a provider with the wallet
    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect(L1_RPC_URL)
        .await
        .map_err(|e| eyre::eyre!("Failed to connect to L1 RPC: {}", e))?;

    // Encode the function call
    let call = sendMessageCall { to, value, message, gasLimit: U256::from(gas_limit) };
    let calldata = call.abi_encode();

    // Build the transaction request
    let tx = TransactionRequest::default()
        .with_to(L1_SCROLL_MESSENGER_PROXY_ADDR)
        .with_input(calldata)
        .with_value(U256::from(10_000_000_000_000_000u64)) // 0.01 ether
        .with_gas_limit(200000)
        .with_gas_price(100_000_000); // 0.1 gwei

    let pending_tx = provider.send_transaction(tx).await?;
    tracing::debug!(
        "📨 Sent L1 Scroll Messenger message to {:?}, tx hash: {:?}",
        to,
        pending_tx.tx_hash()
    );

    if wait_for_confirmation {
        let r = pending_tx.watch().await?;
        tracing::debug!("📨 L1 Scroll Messenger message confirmed: {:?}", r.encode_hex());
    }

    Ok(())
}

/// Send a transaction via the L1 Enforced Transaction Gateway contract.
///
/// # Arguments
/// * `target` - The target address on L2 to call
/// * `value` - The amount of wei to send with the transaction on L2
/// * `gas_limit` - The gas limit for executing the transaction on L2
/// * `data` - The calldata to execute on L2
pub async fn send_l1_enforced_tx_gateway_transaction(
    target: Address,
    value: U256,
    gas_limit: u64,
    data: Bytes,
    wait_for_confirmation: bool,
) -> Result<()> {
    // Parse the private key and create a signer
    let signer: PrivateKeySigner = L1_DEPLOYER_PRIVATE_KEY
        .parse()
        .map_err(|e| eyre::eyre!("Failed to parse L1 deployer private key: {}", e))?;

    // Create a provider with the wallet
    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect(L1_RPC_URL)
        .await
        .map_err(|e| eyre::eyre!("Failed to connect to L1 RPC: {}", e))?;

    // Encode the function call
    let call = sendTransactionCall { target, value, gasLimit: U256::from(gas_limit), data };
    let calldata = call.abi_encode();

    // Build the transaction request
    let tx = TransactionRequest::default()
        .with_to(L1_ENFORCED_TX_GATEWAY_PROXY_ADDR)
        .with_input(calldata)
        .with_value(U256::from(10_000_000_000_000_000u64)) // 0.01 ether
        .with_gas_limit(200000)
        .with_gas_price(100_000_000); // 0.1 gwei

    let pending_tx = provider.send_transaction(tx).await?;
    tracing::debug!(
        "🚀 Sent L1 Enforced Tx Gateway transaction to {:?}, tx hash: {:?}",
        target,
        pending_tx.tx_hash()
    );

    if wait_for_confirmation {
        let r = pending_tx.watch().await?;
        tracing::debug!("🚀 L1 Enforced Tx Gateway transaction confirmed: {:?}", r.encode_hex());
    }

    Ok(())
}

/// Get the L1 message index at the finalized block.
///
/// This function queries the `nextCrossDomainMessageIndex` from the L1 Message Queue V2 contract
/// at the **finalized** block head. The contract returns the next message index, therefore we
/// subtract 1.
///
/// # Returns
/// * `Ok(u64)` - The index of the last L1 messages that has been queued
/// * `Err` - If the call fails or the returned value cannot be converted to u64
pub async fn get_l1_message_index_at_finalized() -> Result<u64> {
    // Create a provider (no signer needed for read-only calls)
    let provider = ProviderBuilder::new()
        .connect(L1_RPC_URL)
        .await
        .map_err(|e| eyre::eyre!("Failed to connect to L1 RPC: {}", e))?;

    // Encode the function call
    let call = nextCrossDomainMessageIndexCall {};
    let calldata = call.abi_encode();

    // Build the call request
    let tx =
        TransactionRequest::default().with_to(L1_MESSAGE_QUEUE_V2_PROXY_ADDR).with_input(calldata);

    // Execute the call at the finalized block
    let result =
        provider.call(tx).block(BlockId::Number(BlockNumberOrTag::Finalized)).await.map_err(
            |e| eyre::eyre!("Failed to call nextCrossDomainMessageIndex at finalized block: {}", e),
        )?;

    // Decode the result - returns U256 directly
    let count_u256 = nextCrossDomainMessageIndexCall::abi_decode_returns(&result)
        .map_err(|e| eyre::eyre!("Failed to decode nextCrossDomainMessageIndex result: {}", e))?;

    // Convert U256 to u64
    let count: u64 =
        count_u256.try_into().map_err(|_| eyre::eyre!("Message count exceeds u64::MAX"))?;

    if count == 0 {
        return Ok(0);
    }

    Ok(count - 1) // Subtract 1 to get the last queued message index
}
