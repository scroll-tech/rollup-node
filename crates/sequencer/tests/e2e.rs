//! e2e tests for the sequencer.

use alloy_consensus::BlockHeader;
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types_engine::ForkchoiceState;
use futures::stream::StreamExt;
use reth_e2e_test_utils::transaction::TransactionTestContext;
use reth_node_core::primitives::SignedTransaction;
use reth_scroll_node::test_utils::setup;
use rollup_node_primitives::L1MessageEnvelope;
use rollup_node_providers::DatabaseL1MessageProvider;
use rollup_node_sequencer::Sequencer;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_provider::ScrollAuthApiEngineClient;
use scroll_db::{test_utils::setup_test_db, DatabaseOperations};
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
    let mut fcs = ForkchoiceState {
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
    let provider = Arc::new(DatabaseL1MessageProvider::new(database.clone(), 0));

    // create a sequencer
    let mut sequencer = Sequencer::new(provider, engine_driver, Default::default(), 4);

    // add a transaction to the pool
    let mut wallet_lock = wallet.lock().await;
    let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
        wallet_lock.chain_id,
        wallet_lock.inner.clone(),
        wallet_lock.inner_nonce,
    )
    .await;
    wallet_lock.inner_nonce += 1;
    drop(wallet_lock);
    let tx_hash = node.rpc.inject_tx(raw_tx).await.unwrap();

    // send a new block request
    sequencer.build_block(fcs);

    // wait for the block to be built
    let block = sequencer.next().await.unwrap();
    let block_1_hash = block.header.hash_slow();

    // make some assertions on the transaction inclusion of the block
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    assert!(block.body.transactions.len() == 1);
    assert!(block.header.number() == 1);
    assert_eq!(block.header.parent_hash, genesis_hash);

    // update the head of the forkchoice state
    fcs.head_block_hash = block.header.hash_slow();

    // now lets add an L1 message to the database
    let wallet_lock = wallet.lock().await;
    let l1_message = L1MessageEnvelope {
        block_number: 1,
        queue_hash: B256::ZERO,
        transaction: TxL1Message {
            queue_index: 0,
            gas_limit: 21000,
            to: Address::random(),
            value: U256::from(1),
            sender: wallet_lock.inner.address(),
            input: vec![].into(),
        },
    };
    drop(wallet_lock);
    let l1_message_hash = l1_message.transaction.tx_hash();
    database.insert_l1_message(l1_message).await.unwrap();

    // sleep 2 seconds (ethereum header timestamp has granularity of seconds and proceeding header
    // must have a greater timestamp than the last)
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // send a new block request this block should include the L1 message
    sequencer.build_block(fcs);

    // wait for the block to be built
    let block = sequencer.next().await.unwrap();

    // make some assertions on the transaction inclusion of the block
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &l1_message_hash);
    assert!(block.body.transactions.len() == 1);
    assert!(block.header.number() == 2);
    assert_eq!(block.header.parent_hash, block_1_hash);
}
