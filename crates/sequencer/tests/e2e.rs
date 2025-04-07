//! e2e tests for the sequencer.

use alloy_consensus::BlockHeader;
use alloy_rpc_types_engine::ForkchoiceState;
use futures::stream::StreamExt;
use reth_e2e_test_utils::transaction::TransactionTestContext;
use reth_node_core::primitives::SignedTransaction;

use reth_scroll_node::test_utils::setup;
use rollup_node_sequencer::Sequencer;
use scroll_alloy_provider::ScrollAuthApiEngineClient;
use scroll_db::test_utils::setup_test_db;
use scroll_engine::{test_utils::NoopExecutionPayloadProvider, EngineDriver};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn can_build_blocks() {
    reth_tracing::init_test_tracing();

    // setup a test node
    let (mut nodes, _tasks, wallet) = setup(1, false).await.unwrap();
    let node = nodes.pop().unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    // create a forkchoice state
    let genesis_hash = node.inner.chain_spec().genesis_hash();
    let fcs = ForkchoiceState {
        head_block_hash: genesis_hash,
        safe_block_hash: genesis_hash,
        finalized_block_hash: genesis_hash,
    };

    // create the engine driver connected to the node
    let auth_client = node.inner.engine_http_client();
    let engine_client = ScrollAuthApiEngineClient::new(auth_client);
    let engine_driver = Arc::new(EngineDriver::new(engine_client, NoopExecutionPayloadProvider));

    // create a test database
    let database = Arc::new(setup_test_db().await);

    // create a sequencer
    let mut sequencer = Sequencer::new(database, engine_driver, 0, 0, 0, 0, 0);

    // add a transaction to the pool
    let mut wallet = wallet.lock().await;
    let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
        wallet.chain_id,
        wallet.inner.clone(),
        wallet.inner_nonce,
    )
    .await;
    wallet.inner_nonce += 1;
    drop(wallet);
    let tx_hash = node.rpc.inject_tx(raw_tx).await.unwrap();

    // send a new block request
    sequencer.build_block(fcs);

    // wait for the block to be built
    let block = sequencer.next().await.unwrap();

    // make some assertions on the transaction inclusion of the block
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    assert!(block.body.transactions.len() == 1);
    assert!(block.header.number() == 1);
}
