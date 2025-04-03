use reth_e2e_test_utils::{transaction::TransactionTestContext, wallet::Wallet};
use reth_scroll_node::test_utils::setup;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn can_build_blocks() {
    reth_tracing::init_test_tracing();

    let (mut nodes, _, wallet) = setup(1, false).await.unwrap();
    let node = nodes.pop().unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    let genesis_hand = node.block_hash(0);

    let mut wallet = wallet.lock().await;
    let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
        wallet.chain_id,
        wallet.inner.clone(),
        wallet.inner_nonce,
    )
    .await;
    wallet.inner_nonce += 1;
    drop(wallet);

    node.rpc.inject_tx(raw_tx).await.unwrap();
}
