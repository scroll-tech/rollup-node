//! e2e tests for the sequencer.

use alloy_consensus::BlockHeader;
use alloy_primitives::{hex, Address, U256};
use alloy_rpc_types_engine::PayloadAttributes;
use futures::stream::StreamExt;
use reth_e2e_test_utils::transaction::TransactionTestContext;
use reth_node_core::primitives::SignedTransaction;
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_node::test_utils::setup;
use rollup_node::{
    test_utils::{default_test_scroll_rollup_node_config, setup_engine},
    BeaconProviderArgs, DatabaseArgs, EngineDriverArgs, L1ProviderArgs, NetworkArgs,
    ScrollRollupNodeConfig, SequencerArgs, SignerArgs,
};
use rollup_node_manager::RollupManagerEvent;
use rollup_node_primitives::{sig_encode_hash, BlockInfo, L1MessageEnvelope};
use rollup_node_providers::{BlobSource, DatabaseL1MessageProvider, ScrollRootProvider};
use rollup_node_sequencer::{L1MessageInclusionMode, Sequencer};
use rollup_node_signer::SignerEvent;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_provider::ScrollAuthApiEngineClient;
use scroll_alloy_rpc_types_engine::{BlockDataHint, ScrollPayloadAttributes};
use scroll_db::{test_utils::setup_test_db, DatabaseOperations};
use scroll_engine::{EngineDriver, EngineDriverEvent, ForkchoiceState};
use std::{
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;
use tokio::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[tokio::test]
async fn can_build_blocks() {
    reth_tracing::init_test_tracing();

    const BLOCK_BUILDING_DURATION: Duration = Duration::from_millis(0);
    const BLOCK_GAP_TRIGGER: u64 = 100;

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
        (*SCROLL_DEV).clone(),
        None::<ScrollRootProvider>,
        fcs,
        false,
        BLOCK_GAP_TRIGGER,
        BLOCK_BUILDING_DURATION,
    );

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = Arc::new(DatabaseL1MessageProvider::new(database.clone(), 0));

    // create a sequencer
    let mut sequencer =
        Sequencer::new(provider, Default::default(), 4, 1, L1MessageInclusionMode::BlockDepth(0));

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
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.header.number(), 1);
    assert_eq!(block.header.parent_hash, genesis_hash);

    // check the base fee has been set for the block.
    assert_eq!(block.header.base_fee_per_gas.unwrap(), 15711571);

    // now lets add an L1 message to the database
    let wallet_lock = wallet.lock().await;
    let l1_message = L1MessageEnvelope {
        l1_block_number: 1,
        l2_block_number: None,
        queue_hash: None,
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
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.header.number(), 2);
    assert_eq!(block.header.parent_hash, block_1_hash);
}

#[tokio::test]
async fn can_build_blocks_with_delayed_l1_messages() {
    reth_tracing::init_test_tracing();

    let chain_spec = SCROLL_DEV.clone();
    const BLOCK_BUILDING_DURATION: Duration = Duration::from_millis(0);
    const BLOCK_GAP_TRIGGER: u64 = 100;
    const L1_MESSAGE_DELAY: u64 = 2;

    // setup a test node
    let (mut nodes, _tasks, wallet) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec, false, false)
            .await
            .unwrap();
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
        (*SCROLL_DEV).clone(),
        None::<ScrollRootProvider>,
        fcs,
        false,
        BLOCK_GAP_TRIGGER,
        BLOCK_BUILDING_DURATION,
    );

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = Arc::new(DatabaseL1MessageProvider::new(database.clone(), 0));

    // create a sequencer
    let mut sequencer = Sequencer::new(
        provider,
        Default::default(),
        4,
        0,
        L1MessageInclusionMode::BlockDepth(L1_MESSAGE_DELAY),
    );

    // now lets add an L1 message to the database (this transaction should not be included until the
    // l1 block number is 3)
    let wallet_lock = wallet.lock().await;
    let l1_message = L1MessageEnvelope {
        l1_block_number: 1,
        l2_block_number: None,
        transaction: TxL1Message {
            queue_index: 0,
            gas_limit: 21000,
            to: Address::random(),
            value: U256::from(1),
            sender: wallet_lock.inner.address(),
            input: vec![].into(),
        },
        queue_hash: None,
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
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.header.number(), 1);
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
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.header.number(), 2);
    assert_eq!(block.header.parent_hash, block_1_hash);
}

#[tokio::test]
async fn can_build_blocks_with_finalized_l1_messages() {
    reth_tracing::init_test_tracing();

    let chain_spec = SCROLL_DEV.clone();
    const BLOCK_BUILDING_DURATION: Duration = tokio::time::Duration::from_millis(0);
    const BLOCK_GAP_TRIGGER: u64 = 100;

    // setup a test node
    let (mut nodes, _tasks, wallet) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec, false, false)
            .await
            .unwrap();
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
        (*SCROLL_DEV).clone(),
        None::<ScrollRootProvider>,
        fcs,
        false,
        BLOCK_GAP_TRIGGER,
        BLOCK_BUILDING_DURATION,
    );

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = Arc::new(DatabaseL1MessageProvider::new(database.clone(), 0));

    // create a sequencer with Finalized mode
    let mut sequencer = Sequencer::new(
        provider,
        Default::default(),
        4,
        5, // current L1 block number
        L1MessageInclusionMode::Finalized,
    );

    // set L1 finalized block number to 2
    sequencer.set_l1_finalized_block_number(2);

    // add L1 messages to database
    let wallet_lock = wallet.lock().await;

    // this message should be included (before finalized block)
    let finalized_l1_message = L1MessageEnvelope {
        l1_block_number: 2, // <= 2 (finalized block)
        l2_block_number: None,
        queue_hash: None,
        transaction: TxL1Message {
            queue_index: 0,
            gas_limit: 21000,
            to: Address::random(),
            value: U256::from(1),
            sender: wallet_lock.inner.address(),
            input: vec![].into(),
        },
    };

    // this message should not be included (after finalized block)
    let unfinalized_l1_message = L1MessageEnvelope {
        l1_block_number: 3, // > 2 (finalized block)
        l2_block_number: None,
        queue_hash: None,
        transaction: TxL1Message {
            queue_index: 1,
            gas_limit: 21000,
            to: Address::random(),
            value: U256::from(2),
            sender: wallet_lock.inner.address(),
            input: vec![].into(),
        },
    };
    drop(wallet_lock);

    let finalized_message_hash = finalized_l1_message.transaction.tx_hash();
    let unfinalized_message_hash = unfinalized_l1_message.transaction.tx_hash();

    database.insert_l1_message(finalized_l1_message).await.unwrap();
    database.insert_l1_message(unfinalized_l1_message).await.unwrap();

    // build payload, should only include finalized message
    sequencer.build_payload_attributes();
    let payload_attributes = sequencer.next().await.unwrap();
    engine_driver.handle_build_new_payload(payload_attributes);

    let block = if let Some(EngineDriverEvent::NewPayload(block)) = engine_driver.next().await {
        block
    } else {
        panic!("expected a new payload event");
    };

    // verify only finalized L1 message is included
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &finalized_message_hash);

    // ensure unfinalized message is not included
    assert!(!block.body.transactions.iter().any(|tx| tx.tx_hash() == &unfinalized_message_hash));

    // update finalized block number to 3, now both messages should be available
    sequencer.set_l1_finalized_block_number(3);

    // sleep 2 seconds (ethereum header timestamp has granularity of seconds and proceeding header
    // must have a greater timestamp than the last)
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // build new payload
    sequencer.build_payload_attributes();
    let payload_attributes = sequencer.next().await.unwrap();
    engine_driver.handle_build_new_payload(payload_attributes);

    let block = if let Some(EngineDriverEvent::NewPayload(block)) = engine_driver.next().await {
        block
    } else {
        panic!("expected a new payload event");
    };

    // now should include the previously unfinalized message
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &unfinalized_message_hash);
}

#[tokio::test]
async fn can_sequence_blocks_with_private_key_file() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create temporary private key file
    let mut temp_file = NamedTempFile::new().unwrap();
    let private_key_hex = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    temp_file.write_all(private_key_hex.as_bytes()).unwrap();
    temp_file.flush().unwrap();

    // Create expected signer
    let expected_key_bytes =
        hex::decode("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
    let expected_signer =
        alloy_signer_local::PrivateKeySigner::from_slice(&expected_key_bytes).unwrap();
    let expected_address = expected_signer.address();

    let chain_spec = (*SCROLL_DEV).clone();
    let rollup_manager_args = ScrollRollupNodeConfig {
        test: false, // disable test mode to enable real signing
        network_args: NetworkArgs::default(),
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: SignerArgs {
            key_file: Some(temp_file.path().to_path_buf()),
            aws_kms_key_id: None,
        },
    };

    let (nodes, _tasks, wallet) =
        setup_engine(rollup_manager_args, 1, chain_spec, false, false).await?;
    let wallet = Arc::new(Mutex::new(wallet));

    let sequencer_rnm_handle = nodes[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await?;

    // Generate and inject transaction
    let mut wallet_lock = wallet.lock().await;
    let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
        wallet_lock.chain_id,
        wallet_lock.inner.clone(),
        wallet_lock.inner_nonce,
    )
    .await;
    wallet_lock.inner_nonce += 1;
    drop(wallet_lock);
    let tx_hash = nodes[0].rpc.inject_tx(raw_tx).await?;

    // Build block
    sequencer_rnm_handle.build_block().await;

    // Verify block was successfully sequenced
    if let Some(RollupManagerEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
        assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    } else {
        panic!("Failed to receive BlockSequenced event");
    }

    // Verify signing event and signature correctness
    if let Some(RollupManagerEvent::SignerEvent(SignerEvent::SignedBlock {
        block: signed_block,
        signature,
    })) = sequencer_events.next().await
    {
        let hash = sig_encode_hash(&signed_block);
        let recovered_address = signature.recover_address_from_prehash(&hash)?;
        assert_eq!(recovered_address, expected_address);
    } else {
        panic!("Failed to receive SignerEvent with signed block");
    }

    Ok(())
}

#[tokio::test]
async fn can_sequence_blocks_with_hex_key_file_without_prefix() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create temporary private key file (without 0x prefix)
    let mut temp_file = NamedTempFile::new().unwrap();
    let private_key_hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    temp_file.write_all(private_key_hex.as_bytes()).unwrap();
    temp_file.flush().unwrap();

    // Create expected signer
    let expected_key_bytes =
        hex::decode("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
    let expected_signer =
        alloy_signer_local::PrivateKeySigner::from_slice(&expected_key_bytes).unwrap();
    let expected_address = expected_signer.address();

    let chain_spec = (*SCROLL_DEV).clone();
    let rollup_manager_args = ScrollRollupNodeConfig {
        test: false, // disable test mode to enable real signing
        network_args: NetworkArgs::default(),
        database_args: DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            block_time: 0,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            ..SequencerArgs::default()
        },
        beacon_provider_args: BeaconProviderArgs {
            blob_source: BlobSource::Mock,
            ..Default::default()
        },
        signer_args: SignerArgs {
            key_file: Some(temp_file.path().to_path_buf()),
            aws_kms_key_id: None,
        },
    };

    let (nodes, _tasks, wallet) =
        setup_engine(rollup_manager_args, 1, chain_spec, false, false).await?;
    let wallet = Arc::new(Mutex::new(wallet));

    let sequencer_rnm_handle = nodes[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await?;

    // Generate and inject transaction
    let mut wallet_lock = wallet.lock().await;
    let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
        wallet_lock.chain_id,
        wallet_lock.inner.clone(),
        wallet_lock.inner_nonce,
    )
    .await;
    wallet_lock.inner_nonce += 1;
    drop(wallet_lock);
    let tx_hash = nodes[0].rpc.inject_tx(raw_tx).await?;

    // Build block
    sequencer_rnm_handle.build_block().await;

    // Verify block was successfully sequenced
    if let Some(RollupManagerEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
        assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    } else {
        panic!("Failed to receive BlockSequenced event");
    }

    // Verify signing event and signature correctness
    if let Some(RollupManagerEvent::SignerEvent(SignerEvent::SignedBlock {
        block: signed_block,
        signature,
    })) = sequencer_events.next().await
    {
        let hash = sig_encode_hash(&signed_block);
        let recovered_address = signature.recover_address_from_prehash(&hash)?;
        assert_eq!(recovered_address, expected_address);
    } else {
        panic!("Failed to receive SignerEvent with signed block");
    }

    Ok(())
}

#[tokio::test]
async fn can_build_blocks_and_exit_at_gas_limit() {
    reth_tracing::init_test_tracing();

    let chain_spec = SCROLL_DEV.clone();
    const MIN_TRANSACTION_GAS_COST: u64 = 21_000;
    const BLOCK_BUILDING_DURATION: Duration = Duration::from_millis(250);
    const BLOCK_GAP_TRIGGER: u64 = 100;
    const TRANSACTIONS_COUNT: usize = 2000;

    // setup a test node. use a high value for the payload building duration to be sure we don't
    // exit early.
    let (mut nodes, _tasks, wallet) = setup_engine(
        ScrollRollupNodeConfig {
            sequencer_args: SequencerArgs { payload_building_duration: 1000, ..Default::default() },
            ..default_test_scroll_rollup_node_config()
        },
        1,
        chain_spec,
        false,
        false,
    )
    .await
    .unwrap();
    let node = nodes.pop().unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    // add transactions.
    let mut wallet_lock = wallet.lock().await;
    for _ in 0..TRANSACTIONS_COUNT {
        let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
            wallet_lock.chain_id,
            wallet_lock.inner.clone(),
            wallet_lock.inner_nonce,
        )
        .await;
        wallet_lock.inner_nonce += 1;
        node.rpc.inject_tx(raw_tx).await.unwrap();
    }
    drop(wallet_lock);

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
        (*SCROLL_DEV).clone(),
        None::<ScrollRootProvider>,
        fcs,
        false,
        BLOCK_GAP_TRIGGER,
        BLOCK_BUILDING_DURATION,
    );

    // issue a new payload to the execution layer.
    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).expect("Time can't go backwards").as_secs();
    engine_driver.handle_build_new_payload(ScrollPayloadAttributes {
        payload_attributes: PayloadAttributes {
            timestamp,
            prev_randao: Default::default(),
            suggested_fee_recipient: Default::default(),
            withdrawals: None,
            parent_beacon_block_root: None,
        },
        transactions: None,
        no_tx_pool: false,
        block_data_hint: BlockDataHint::none(),
        gas_limit: None,
    });

    // verify the gas used is within MIN_TRANSACTION_GAS_COST of the gas limit.
    if let Some(EngineDriverEvent::NewPayload(block)) = engine_driver.next().await {
        assert!(block.header.gas_used >= block.gas_limit - MIN_TRANSACTION_GAS_COST);
    } else {
        panic!("expected a new payload event");
    }
}

#[tokio::test]
async fn can_build_blocks_and_exit_at_time_limit() {
    reth_tracing::init_test_tracing();

    let chain_spec = SCROLL_DEV.clone();
    const MIN_TRANSACTION_GAS_COST: u64 = 21_000;
    const BLOCK_BUILDING_DURATION: Duration = Duration::from_secs(1);
    const BLOCK_GAP_TRIGGER: u64 = 100;
    const TRANSACTIONS_COUNT: usize = 2000;

    // setup a test node. use a low payload building duration in order to exit before we reach the
    // gas limit.
    let (mut nodes, _tasks, wallet) = setup_engine(
        ScrollRollupNodeConfig {
            sequencer_args: SequencerArgs { payload_building_duration: 10, ..Default::default() },
            ..default_test_scroll_rollup_node_config()
        },
        1,
        chain_spec,
        false,
        false,
    )
    .await
    .unwrap();
    let node = nodes.pop().unwrap();
    let wallet = Arc::new(Mutex::new(wallet));

    // add transactions.
    let mut wallet_lock = wallet.lock().await;
    for _ in 0..TRANSACTIONS_COUNT {
        let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
            wallet_lock.chain_id,
            wallet_lock.inner.clone(),
            wallet_lock.inner_nonce,
        )
        .await;
        wallet_lock.inner_nonce += 1;
        node.rpc.inject_tx(raw_tx).await.unwrap();
    }
    drop(wallet_lock);

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
        (*SCROLL_DEV).clone(),
        None::<ScrollRootProvider>,
        fcs,
        false,
        BLOCK_GAP_TRIGGER,
        BLOCK_BUILDING_DURATION,
    );

    // start timer.
    let start = Instant::now();

    // issue a new payload to the execution layer.
    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).expect("Time can't go backwards").as_secs();
    engine_driver.handle_build_new_payload(ScrollPayloadAttributes {
        payload_attributes: PayloadAttributes {
            timestamp,
            prev_randao: Default::default(),
            suggested_fee_recipient: Default::default(),
            withdrawals: None,
            parent_beacon_block_root: None,
        },
        transactions: None,
        no_tx_pool: false,
        block_data_hint: BlockDataHint::none(),
        gas_limit: None,
    });

    if let Some(EngineDriverEvent::NewPayload(block)) = engine_driver.next().await {
        let payload_building_duration = start.elapsed();
        // verify that the block building duration is within 10% of the target (we allow for 10%
        // mismatch due to slower performance of debug mode).
        assert!(payload_building_duration < BLOCK_BUILDING_DURATION * 110 / 100);
        assert!(block.gas_used < block.gas_limit - MIN_TRANSACTION_GAS_COST);
    } else {
        panic!("expected a new payload event");
    }
}
