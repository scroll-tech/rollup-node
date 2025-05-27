//! Contains tests related to RN and EN sync.

use alloy_consensus::BlockBody;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Bytes, Signature};
use alloy_provider::{Provider, ProviderBuilder};
use reth_e2e_test_utils::{transaction::TransactionTestContext, wallet::Wallet, NodeHelperType};
use reth_eth_wire_types::NewBlock;
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_primitives::{ScrollBlock, ScrollTransactionSigned};
use rollup_node::{
    test_utils::{default_test_scroll_rollup_node_config, setup_engine},
    ScrollRollupNode,
};
use rollup_node_manager::RollupManagerCommand;
use scroll_alloy_network::Scroll;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock},
};
use tokio::sync::{oneshot, Mutex};

static WALLET: LazyLock<Arc<Mutex<Wallet>>> = LazyLock::new(|| {
    let chain_spec = (*SCROLL_DEV).clone();
    Arc::new(Mutex::new(Wallet::default().with_chain_id(chain_spec.chain().into())))
});

/// We test if the syncing of the RN is correctly triggered and released when the EN reaches sync.
#[allow(clippy::large_stack_frames)]
#[tokio::test]
async fn can_sync_en() {
    reth_tracing::init_test_tracing();
    let node_config = default_test_scroll_rollup_node_config();

    // Create the chain spec for scroll mainnet with Euclid v2 activated and a test genesis.
    let chain_spec = (*SCROLL_DEV).clone();
    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec.clone(), false).await.unwrap();
    let mut synced = nodes.pop().unwrap();

    let (mut nodes, _tasks, _) =
        setup_engine(node_config.clone(), 1, chain_spec, false).await.unwrap();
    let mut unsynced = nodes.pop().unwrap();

    // Advance the chain.
    synced.advance(node_config.engine_driver_args.en_sync_trigger + 1, tx_gen).await.unwrap();

    // Connect the nodes together.
    synced.network.add_peer(unsynced.network.record()).await;
    unsynced.network.next_session_established().await;
    synced.network.next_session_established().await;

    // Announce the latest block.
    announce_latest_block(&synced).await;

    // Check the unsynced node enters sync mode.
    let (tx, rx) = oneshot::channel();
    unsynced
        .inner
        .add_ons_handle
        .rollup_manager_handle
        .send_command(RollupManagerCommand::Status(tx))
        .await;
    let status = rx.await.unwrap();
    assert!(status.syncing);

    // Verify the unsynced node syncs.
    let provider = ProviderBuilder::new().connect_http(unsynced.rpc_url());
    let mut retries = 0;
    let mut num = provider.get_block_number().await.unwrap();

    loop {
        if retries > 20 || num > node_config.engine_driver_args.en_sync_trigger {
            break
        }
        num = provider.get_block_number().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        retries += 1;
    }

    // Build and announce a new block to have engine driver exit syncing mode.
    build_block_and_announce(&mut synced).await;

    // Check the unsynced node exits sync mode.
    let (tx, rx) = oneshot::channel();
    unsynced
        .inner
        .add_ons_handle
        .rollup_manager_handle
        .send_command(RollupManagerCommand::Status(tx))
        .await;
    let status = rx.await.unwrap();
    assert!(!status.syncing);
}

fn tx_gen(_: u64) -> Pin<Box<dyn Future<Output = Bytes>>> {
    let wallet = WALLET.clone();
    Box::pin(async move {
        let mut wallet = wallet.lock().await;
        let tx_fut = TransactionTestContext::transfer_tx_nonce_bytes(
            wallet.chain_id,
            wallet.inner.clone(),
            wallet.inner_nonce,
        );
        wallet.inner_nonce += 1;
        tx_fut.await
    })
}

/// Builds a block and announces it on the network.
async fn build_block_and_announce(node: &mut NodeHelperType<ScrollRollupNode>) {
    node.advance(1, tx_gen).await.unwrap();
    announce_latest_block(node).await;
}

/// Announce the latest block on the network.
async fn announce_latest_block(node: &NodeHelperType<ScrollRollupNode>) {
    // Fetch the latest block.
    let provider = ProviderBuilder::<_, _, Scroll>::default()
        .with_recommended_fillers()
        .connect_http(node.rpc_url());
    let latest_block = provider
        .get_block(BlockId::Number(BlockNumberOrTag::Latest))
        .full()
        .await
        .unwrap()
        .unwrap();

    // Convert the block to alloy_consensus::Block and announce it.
    let latest_block_hash = latest_block.header.hash;
    let transactions = latest_block
        .transactions
        .into_transactions()
        .map(|tx| {
            let signed = tx.inner.inner.into_inner();
            let signature = signed
                .signature()
                .unwrap_or_else(|| Signature::try_from([0u8; 65].as_slice()).unwrap());
            ScrollTransactionSigned::new(signed.into(), signature)
        })
        .collect();
    let mut latest_block = ScrollBlock::new(
        latest_block.header.into(),
        BlockBody { transactions, ..Default::default() },
    );
    latest_block.header.extra_data = Bytes::from([0u8; 65].as_slice());
    let latest_block = NewBlock { block: latest_block, ..Default::default() };
    node.inner.network.announce_block(latest_block, latest_block_hash);

    // Wait for block to be propagated.
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}
