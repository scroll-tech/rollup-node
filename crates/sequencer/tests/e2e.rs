//! e2e tests for the sequencer.

use alloy_consensus::BlockHeader;
use alloy_primitives::{Address, U256};
use futures::stream::StreamExt;
use reth_e2e_test_utils::transaction::TransactionTestContext;
use reth_node_core::primitives::SignedTransaction;
use reth_scroll_node::test_utils::setup;
use rollup_node_primitives::{BlockInfo, L1MessageWithBlockNumber};
use rollup_node_providers::DatabaseL1MessageProvider;
use rollup_node_sequencer::Sequencer;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_provider::ScrollAuthApiEngineClient;
use scroll_db::{test_utils::setup_test_db, DatabaseOperations};
use scroll_engine::{
    test_utils::NoopExecutionPayloadProvider, EngineDriver, EngineDriverEvent, ForkchoiceState,
};
use std::sync::Arc;
use tokio::{sync::Mutex, time::Duration};

#[tokio::test]
async fn can_build_blocks() {
    reth_tracing::init_test_tracing();

    const BLOCK_BUILDING_DURATION: Duration = tokio::time::Duration::from_millis(0);

    // setup a test node
    let (mut nodes, _tasks, wallet) = setup(1, false).await.unwrap();
    let node = nodes.pop().unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    // create a forkchoice state
    let genesis_hash = node.inner.chain_spec().genesis_hash();
    let fcs = ForkchoiceState::new(
        BlockInfo { hash: genesis_hash, number: 0 },
        Default::default(),
        Default::default(),
    );

    // create the engine driver connected to the node
    let auth_client = node.inner.engine_http_client();
    let engine_client = ScrollAuthApiEngineClient::new(auth_client);
    let mut engine_driver = EngineDriver::new(
        Arc::new(engine_client),
        Arc::new(NoopExecutionPayloadProvider),
        fcs,
        BLOCK_BUILDING_DURATION,
    );

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = Arc::new(DatabaseL1MessageProvider::new(database.clone(), 0));

    // create a sequencer
    let mut sequencer = Sequencer::new(provider, Default::default(), 4, 1, 0);

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

    // send a new payload attributes request.
    sequencer.build_payload_attributes();
    let payload_attributes = sequencer.next().await.unwrap();
    engine_driver.handle_build_new_payload(payload_attributes);

    let block = if let Some(EngineDriverEvent::NewPayload(block)) = engine_driver.next().await {
        block
    } else {
        panic!("expected a new payload event");
    };

    // wait for the block to be built
    let block_1_hash = block.header.hash_slow();

    // make some assertions on the transaction inclusion of the block
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    assert!(block.body.transactions.len() == 1);
    assert!(block.header.number() == 1);
    assert_eq!(block.header.parent_hash, genesis_hash);

    // now lets add an L1 message to the database
    let wallet_lock = wallet.lock().await;
    let l1_message = L1MessageWithBlockNumber {
        block_number: 1,
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
    sequencer.build_payload_attributes();
    let payload_attributes = sequencer.next().await.unwrap();
    engine_driver.handle_build_new_payload(payload_attributes);

    let block = if let Some(EngineDriverEvent::NewPayload(block)) = engine_driver.next().await {
        block
    } else {
        panic!("expected a new payload event");
    };

    // make some assertions on the transaction inclusion of the block
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &l1_message_hash);
    assert!(block.body.transactions.len() == 1);
    assert!(block.header.number() == 2);
    assert_eq!(block.header.parent_hash, block_1_hash);
}

#[tokio::test]
async fn can_build_blocks_with_delayed_l1_messages() {
    reth_tracing::init_test_tracing();

    const BLOCK_BUILDING_DURATION: Duration = tokio::time::Duration::from_millis(0);
    const L1_MESSAGE_DELAY: u64 = 2;

    // setup a test node
    let (mut nodes, _tasks, wallet) = setup(1, false).await.unwrap();
    let node = nodes.pop().unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    // create a forkchoice state
    let genesis_hash = node.inner.chain_spec().genesis_hash();
    let fcs = ForkchoiceState::new(
        BlockInfo { hash: genesis_hash, number: 0 },
        Default::default(),
        Default::default(),
    );

    // create the engine driver connected to the node
    let auth_client = node.inner.engine_http_client();
    let engine_client = ScrollAuthApiEngineClient::new(auth_client);
    let mut engine_driver = EngineDriver::new(
        Arc::new(engine_client),
        Arc::new(NoopExecutionPayloadProvider),
        fcs,
        BLOCK_BUILDING_DURATION,
    );

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = Arc::new(DatabaseL1MessageProvider::new(database.clone(), 0));

    // create a sequencer
    let mut sequencer = Sequencer::new(provider, Default::default(), 4, 0, L1_MESSAGE_DELAY);

    // now lets add an L1 message to the database (this transaction should not be included until the
    // l1 block number is 3)
    let wallet_lock = wallet.lock().await;
    let l1_message = L1MessageWithBlockNumber {
        block_number: 1,
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

    // send a new payload attributes request.
    sequencer.build_payload_attributes();
    let payload_attributes = sequencer.next().await.unwrap();
    engine_driver.handle_build_new_payload(payload_attributes);

    let block = if let Some(EngineDriverEvent::NewPayload(block)) = engine_driver.next().await {
        block
    } else {
        panic!("expected a new payload event");
    };

    // wait for the block to be built
    let block_1_hash = block.header.hash_slow();

    // make some assertions on the transaction inclusion of the block
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    assert!(block.body.transactions.len() == 1);
    assert!(block.header.number() == 1);
    assert_eq!(block.header.parent_hash, genesis_hash);

    // sleep 2 seconds (ethereum header timestamp has granularity of seconds and proceeding header
    // must have a greater timestamp than the last)
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // set the l1 block number to 3
    sequencer.handle_new_l1_block(3);

    // send a new block request this block should include the L1 message
    sequencer.build_payload_attributes();
    let payload_attributes = sequencer.next().await.unwrap();
    engine_driver.handle_build_new_payload(payload_attributes);

    let block = if let Some(EngineDriverEvent::NewPayload(block)) = engine_driver.next().await {
        block
    } else {
        panic!("expected a new payload event");
    };

    // make some assertions on the transaction inclusion of the block
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &l1_message_hash);
    assert!(block.body.transactions.len() == 1);
    assert!(block.header.number() == 2);
    assert_eq!(block.header.parent_hash, block_1_hash);
}
