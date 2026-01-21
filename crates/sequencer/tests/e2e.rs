//! e2e tests for the sequencer.

use alloy_consensus::{transaction::TxHashRef, BlockHeader};
use alloy_primitives::{hex, Address, U256};
use futures::stream::StreamExt;
use reth_e2e_test_utils::transaction::TransactionTestContext;
use reth_scroll_chainspec::SCROLL_DEV;
use reth_scroll_node::test_utils::setup;
use rollup_node::{
    constants::SCROLL_GAS_LIMIT,
    test_utils::{default_test_scroll_rollup_node_config, setup_engine},
    BlobProviderArgs, ChainOrchestratorArgs, ConsensusArgs, EngineDriverArgs, L1ProviderArgs,
    PprofArgs, RollupNodeDatabaseArgs, RollupNodeGasPriceOracleArgs, RollupNodeNetworkArgs,
    RpcArgs, ScrollRollupNodeConfig, SequencerArgs, SignerArgs,
};
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use rollup_node_primitives::{sig_encode_hash, BlockInfo, L1MessageEnvelope};
use rollup_node_sequencer::{
    L1MessageInclusionMode, PayloadBuildingConfig, Sequencer, SequencerConfig, SequencerEvent,
};
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::{ScrollTransaction, TxL1Message};
use scroll_alloy_provider::ScrollAuthApiEngineClient;
use scroll_db::{test_utils::setup_test_db, DatabaseWriteOperations};
use scroll_engine::{Engine, ForkchoiceState};
use std::{io::Write, path::PathBuf, sync::Arc};
use tempfile::NamedTempFile;
use tokio::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[tokio::test]
async fn skip_block_with_no_transactions() {
    reth_tracing::init_test_tracing();

    // setup a test node
    let (mut nodes, _tasks, _wallet) = setup(1, false).await.unwrap();
    let node = nodes.pop().unwrap();

    // create a fork choice state
    let genesis_hash = node.inner.chain_spec().genesis_hash();
    let fcs = ForkchoiceState::new(
        BlockInfo { hash: genesis_hash, number: 0 },
        Default::default(),
        Default::default(),
    );

    // create the engine driver connected to the node
    let auth_client = node.inner.engine_http_client();
    let engine_client = ScrollAuthApiEngineClient::new(auth_client);
    let mut engine = Engine::new(Arc::new(engine_client), fcs);

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = database.clone();

    // Set the latest block number
    database.set_latest_l1_block_number(0).await.unwrap();

    // create a sequencer
    let config = SequencerConfig {
        chain_spec: node.inner.chain_spec(),
        fee_recipient: Address::random(),
        auto_start: false,
        payload_building_config: PayloadBuildingConfig {
            block_gas_limit: SCROLL_GAS_LIMIT,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
        },
        block_time: 1,
        payload_building_duration: 0,
        allow_empty_blocks: false,
    };
    let mut sequencer = Sequencer::new(provider, config);

    // send a new payload attributes request.
    sequencer.start_payload_building(&mut engine).await.unwrap();
    if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_none(), "expected no new payload, but got: {:?}", result);
    } else {
        panic!("expected a payload ready event");
    };
}

#[tokio::test]
async fn can_build_blocks() {
    reth_tracing::init_test_tracing();

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
    let mut engine = Engine::new(Arc::new(engine_client), fcs);

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = database.clone();

    // Set the latest block number
    database.set_latest_l1_block_number(5).await.unwrap();

    // create a sequencer
    let config = SequencerConfig {
        chain_spec: node.inner.chain_spec(),
        fee_recipient: Address::random(),
        auto_start: false,
        payload_building_config: PayloadBuildingConfig {
            block_gas_limit: SCROLL_GAS_LIMIT,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
        },
        block_time: 1,
        payload_building_duration: 0,
        allow_empty_blocks: true,
    };
    let mut sequencer = Sequencer::new(provider, config);

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
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_some(), "expected a new payload, but got: {:?}", result);
        result.unwrap()
    } else {
        panic!("expected a payload ready event");
    };

    // wait for the block to be built
    let block_1_hash = block.header.hash_slow();

    // make some assertions on the transaction inclusion of the block
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.header.number(), 1);
    assert_eq!(block.header.parent_hash, genesis_hash);

    // check the base fee has been set for the block.
    assert_eq!(block.header.base_fee_per_gas.unwrap(), 876960000);

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
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_some(), "expected a new payload, but got: {:?}", result);
        result.unwrap()
    } else {
        panic!("expected a payload ready event");
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
    const L1_MESSAGE_DELAY: u64 = 2;

    // setup a test node
    let (mut nodes, _, wallet) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec, false, false, None)
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
    let mut engine = Engine::new(Arc::new(engine_client), fcs);

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = database.clone();

    // Set the latest block number
    database.set_latest_l1_block_number(1).await.unwrap();

    // create a sequencer
    let config = SequencerConfig {
        chain_spec: node.inner.chain_spec(),
        fee_recipient: Address::random(),
        auto_start: false,
        payload_building_config: PayloadBuildingConfig {
            block_gas_limit: SCROLL_GAS_LIMIT,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(L1_MESSAGE_DELAY),
        },
        block_time: 0,
        payload_building_duration: 0,
        allow_empty_blocks: true,
    };
    let mut sequencer = Sequencer::new(provider, config);

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
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_some(), "expected a new payload, but got: {:?}", result);
        result.unwrap()
    } else {
        panic!("expected a payload ready event");
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
    database.set_latest_l1_block_number(3).await.unwrap();

    // send a new block request this block should include the L1 message
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_some(), "expected a new payload, but got: {:?}", result);
        result.unwrap()
    } else {
        panic!("expected a payload ready event");
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
    // setup a test node
    let (mut nodes, _, wallet) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec, false, false, None)
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
    let mut engine = Engine::new(Arc::new(engine_client), fcs);

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = database.clone();

    database.set_latest_l1_block_number(5).await.unwrap();

    // create a sequencer
    let config = SequencerConfig {
        chain_spec: node.inner.chain_spec(),
        fee_recipient: Address::random(),
        auto_start: false,
        payload_building_config: PayloadBuildingConfig {
            block_gas_limit: SCROLL_GAS_LIMIT,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::FinalizedWithBlockDepth(0),
        },
        block_time: 0,
        payload_building_duration: 0,
        allow_empty_blocks: true,
    };
    let mut sequencer = Sequencer::new(provider, config);

    // set L1 finalized block number to 2
    database.set_finalized_l1_block_number(2).await.unwrap();

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
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_some(), "expected a new payload, but got: {:?}", result);
        result.unwrap()
    } else {
        panic!("expected a payload ready event");
    };

    // verify only finalized L1 message is included
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &finalized_message_hash);

    // ensure unfinalized message is not included
    assert!(!block.body.transactions.iter().any(|tx| tx.tx_hash() == &unfinalized_message_hash));

    // Handle the build block with the sequencer in order to update L1 message queue index.
    database.update_l1_messages_with_l2_blocks(vec![(&block).into()]).await.unwrap();

    // update finalized block number to 3, now both messages should be available
    database.set_finalized_l1_block_number(3).await.unwrap();

    // sleep 2 seconds (ethereum header timestamp has granularity of seconds and proceeding header
    // must have a greater timestamp than the last)
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // build new payload
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_some(), "expected a new payload, but got: {:?}", result);
        result.unwrap()
    } else {
        panic!("expected a payload ready event");
    };

    // now should include the previously unfinalized message
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &unfinalized_message_hash);
}

#[tokio::test]
async fn can_sequence_blocks_with_private_key_file() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    // Create temporary private key file
    let mut temp_file = NamedTempFile::new()?;
    let private_key_hex = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    temp_file.write_all(private_key_hex.as_bytes())?;
    temp_file.flush()?;

    // Create expected signer
    let expected_key_bytes =
        hex::decode("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")?;
    let expected_signer = alloy_signer_local::PrivateKeySigner::from_slice(&expected_key_bytes)?;
    let expected_address = expected_signer.address();

    let chain_spec = (*SCROLL_DEV).clone();
    let rollup_manager_args = ScrollRollupNodeConfig {
        test_args: TestArgs {
            test: false, // disable test mode to enable real signing
            skip_l1_synced: false,
        },
        network_args: RollupNodeNetworkArgs::default(),
        database_args: RollupNodeDatabaseArgs {
            rn_db_path: Some(PathBuf::from("sqlite::memory:")),
        },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            auto_start: false,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            allow_empty_blocks: true,
            ..SequencerArgs::default()
        },
        blob_provider_args: BlobProviderArgs { mock: true, ..Default::default() },
        signer_args: SignerArgs {
            key_file: Some(temp_file.path().to_path_buf()),
            aws_kms_key_id: None,
            private_key: None,
        },
        gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
        rpc_args: RpcArgs::default(),
        pprof_args: PprofArgs::default(),
    };

    let (nodes, _, wallet) =
        setup_engine(rollup_manager_args, 1, chain_spec, false, false, None).await?;
    let wallet = Arc::new(Mutex::new(wallet));

    let sequencer_rnm_handle = nodes[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await?;
    let sequencer_l1_watcher_tx =
        nodes[0].inner.add_ons_handle.rollup_manager_handle.l1_watcher_mock.clone().unwrap();

    // Send a notification to set the L1 to synced
    sequencer_l1_watcher_tx.notification_tx.send(Arc::new(L1Notification::Synced)).await?;

    // skip the L1 synced event and consolidated events
    sequencer_events.next().await;
    sequencer_events.next().await;

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
    sequencer_rnm_handle.build_block();

    // Verify block was successfully sequenced
    if let Some(ChainOrchestratorEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
        assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    } else {
        panic!("Failed to receive BlockSequenced event");
    }

    // Verify signing event and signature correctness
    if let Some(ChainOrchestratorEvent::SignedBlock { block: signed_block, signature }) =
        sequencer_events.next().await
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
        test_args: TestArgs {
            test: false, // disable test mode to enable real signing
            skip_l1_synced: false,
        },
        network_args: RollupNodeNetworkArgs::default(),
        database_args: RollupNodeDatabaseArgs {
            rn_db_path: Some(PathBuf::from("sqlite::memory:")),
        },
        l1_provider_args: L1ProviderArgs::default(),
        engine_driver_args: EngineDriverArgs::default(),
        chain_orchestrator_args: ChainOrchestratorArgs::default(),
        sequencer_args: SequencerArgs {
            sequencer_enabled: true,
            auto_start: false,
            block_time: 0,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
            payload_building_duration: 1000,
            allow_empty_blocks: true,
            ..SequencerArgs::default()
        },
        blob_provider_args: BlobProviderArgs { mock: true, ..Default::default() },
        signer_args: SignerArgs {
            key_file: Some(temp_file.path().to_path_buf()),
            aws_kms_key_id: None,
            private_key: None,
        },
        gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
        consensus_args: ConsensusArgs::noop(),
        database: None,
        rpc_args: RpcArgs::default(),
        pprof_args: PprofArgs::default(),
    };

    let (nodes, _, wallet) =
        setup_engine(rollup_manager_args, 1, chain_spec, false, false, None).await?;
    let wallet = Arc::new(Mutex::new(wallet));

    let sequencer_rnm_handle = nodes[0].inner.add_ons_handle.rollup_manager_handle.clone();
    let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await?;
    let sequencer_l1_watcher_tx =
        nodes[0].inner.add_ons_handle.rollup_manager_handle.l1_watcher_mock.clone().unwrap();

    // Send a notification to set the L1 to synced
    sequencer_l1_watcher_tx.notification_tx.send(Arc::new(L1Notification::Synced)).await?;

    // skip the L1 synced event and consolidated events
    sequencer_events.next().await;
    sequencer_events.next().await;

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
    sequencer_rnm_handle.build_block();

    // Verify block was successfully sequenced
    if let Some(ChainOrchestratorEvent::BlockSequenced(block)) = sequencer_events.next().await {
        assert_eq!(block.body.transactions.len(), 1);
        assert_eq!(block.body.transactions.first().unwrap().tx_hash(), &tx_hash);
    } else {
        panic!("Failed to receive BlockSequenced event");
    }

    // Verify signing event and signature correctness
    while let Some(event) = sequencer_events.next().await {
        if let ChainOrchestratorEvent::SignedBlock { block: signed_block, signature } = event {
            let hash = sig_encode_hash(&signed_block);
            let recovered_address = signature.recover_address_from_prehash(&hash)?;
            assert_eq!(recovered_address, expected_address);
            break;
        }
    }

    Ok(())
}

#[tokio::test]
async fn can_build_blocks_and_exit_at_gas_limit() {
    reth_tracing::init_test_tracing();

    let chain_spec = SCROLL_DEV.clone();
    const MIN_TRANSACTION_GAS_COST: u64 = 21_000;
    const TRANSACTIONS_COUNT: usize = 2000;

    // setup a test node. use a high value for the payload building duration to be sure we don't
    // exit early.
    let (mut nodes, _, wallet) = setup_engine(
        ScrollRollupNodeConfig {
            sequencer_args: SequencerArgs { payload_building_duration: 1000, ..Default::default() },
            ..default_test_scroll_rollup_node_config()
        },
        1,
        chain_spec,
        false,
        false,
        None,
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
    let mut engine = Engine::new(Arc::new(engine_client), fcs);

    // create a test database
    let database = Arc::new(setup_test_db().await);

    // create a sequencer
    let config = SequencerConfig {
        chain_spec: node.inner.chain_spec(),
        fee_recipient: Address::random(),
        auto_start: false,
        payload_building_config: PayloadBuildingConfig {
            block_gas_limit: SCROLL_GAS_LIMIT,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
        },
        block_time: 1,
        payload_building_duration: 0,
        allow_empty_blocks: false,
    };
    let mut sequencer = Sequencer::new(database, config);

    // build a new payload
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_some(), "expected a new payload, but got: {:?}", result);
        result.unwrap()
    } else {
        panic!("expected a payload ready event");
    };

    // verify the gas used is within MIN_TRANSACTION_GAS_COST of the gas limit.
    assert!(block.header.gas_used >= block.gas_limit - MIN_TRANSACTION_GAS_COST);
}

#[tokio::test]
async fn can_build_blocks_and_exit_at_time_limit() {
    reth_tracing::init_test_tracing();

    let chain_spec = SCROLL_DEV.clone();
    const MIN_TRANSACTION_GAS_COST: u64 = 21_000;
    const BLOCK_BUILDING_DURATION: Duration = Duration::from_secs(1);
    const TRANSACTIONS_COUNT: usize = 2000;

    // setup a test node. use a low payload building duration in order to exit before we reach the
    // gas limit.
    let (mut nodes, _, wallet) = setup_engine(
        ScrollRollupNodeConfig {
            sequencer_args: SequencerArgs { payload_building_duration: 10, ..Default::default() },
            ..default_test_scroll_rollup_node_config()
        },
        1,
        chain_spec,
        false,
        false,
        None,
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
    let mut engine = Engine::new(Arc::new(engine_client), fcs);

    // create a test database
    let database = Arc::new(setup_test_db().await);

    // create a sequencer
    let config = SequencerConfig {
        chain_spec: node.inner.chain_spec(),
        fee_recipient: Address::random(),
        auto_start: false,
        payload_building_config: PayloadBuildingConfig {
            block_gas_limit: SCROLL_GAS_LIMIT,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::BlockDepth(0),
        },
        block_time: 1,
        payload_building_duration: 0,
        allow_empty_blocks: false,
    };
    let mut sequencer = Sequencer::new(database, config);

    // start timer.
    let start = Instant::now();

    // issue a new payload to the execution layer.
    // build a new payload
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let result = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(result.is_some(), "expected a new payload, but got: {:?}", result);
        result.unwrap()
    } else {
        panic!("expected a payload ready event");
    };

    let payload_building_duration = start.elapsed();
    // verify that the block building duration is within 10% of the target (we allow for 10%
    // mismatch due to slower performance of debug mode).
    assert!(payload_building_duration < BLOCK_BUILDING_DURATION * 110 / 100);
    assert!(block.gas_used < block.gas_limit - MIN_TRANSACTION_GAS_COST);
}

#[tokio::test]
async fn should_limit_l1_message_cumulative_gas() {
    reth_tracing::init_test_tracing();

    // setup a test node
    let chain_spec = SCROLL_DEV.clone();
    let (mut nodes, _, wallet) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec, false, false, None)
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
    let mut engine = Engine::new(Arc::new(engine_client), fcs);

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = database.clone();

    // Set the latest and finalized block number
    database.set_latest_l1_block_number(5).await.unwrap();
    database.set_finalized_l1_block_number(1).await.unwrap();

    // create a sequencer
    let config = SequencerConfig {
        chain_spec: node.inner.chain_spec(),
        fee_recipient: Address::random(),
        auto_start: false,
        payload_building_config: PayloadBuildingConfig {
            block_gas_limit: SCROLL_GAS_LIMIT,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::FinalizedWithBlockDepth(0),
        },
        block_time: 0,
        payload_building_duration: 0,
        allow_empty_blocks: true,
    };
    let mut sequencer = Sequencer::new(provider, config);

    // add L1 messages to database
    let wallet_lock = wallet.lock().await;
    let l1_messages = [
        L1MessageEnvelope {
            l1_block_number: 1,
            l2_block_number: None,
            queue_hash: None,
            transaction: TxL1Message {
                queue_index: 0,
                gas_limit: SCROLL_GAS_LIMIT / 2,
                to: Address::random(),
                value: U256::from(1),
                sender: wallet_lock.inner.address(),
                input: vec![].into(),
            },
        },
        L1MessageEnvelope {
            l1_block_number: 1,
            l2_block_number: None,
            queue_hash: None,
            transaction: TxL1Message {
                queue_index: 1,
                gas_limit: SCROLL_GAS_LIMIT / 2 + 1,
                to: Address::random(),
                value: U256::from(1),
                sender: wallet_lock.inner.address(),
                input: vec![].into(),
            },
        },
    ];
    for l1_message in l1_messages {
        database.insert_l1_message(l1_message).await.unwrap();
    }

    // build payload, should only include first l1 message
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let block = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(block.is_some(), "expected a new payload, but got: {:?}", block);
        block.unwrap()
    } else {
        panic!("expected a payload ready event");
    };

    // verify only one L1 message is included
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.header.gas_used, 21_000);

    // sleep 1 seconds (ethereum header timestamp has granularity of seconds and proceeding header
    // must have a greater timestamp than the last)
    tokio::time::sleep(Duration::from_secs(1)).await;

    // build new payload
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let block = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(block.is_some(), "expected a new payload, but got: {:?}", block);
        block.unwrap()
    } else {
        panic!("expected a payload ready event");
    };

    // now should include the next l1 message.
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(block.header.gas_used(), 21_000);
}

#[tokio::test]
async fn should_not_add_skipped_messages() {
    reth_tracing::init_test_tracing();

    // setup a test node
    let chain_spec = SCROLL_DEV.clone();
    let (mut nodes, _, wallet) =
        setup_engine(default_test_scroll_rollup_node_config(), 1, chain_spec, false, false, None)
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
    let mut engine = Engine::new(Arc::new(engine_client), fcs);

    // create a test database
    let database = Arc::new(setup_test_db().await);
    let provider = database.clone();

    // Set the latest and finalized block number
    database.set_latest_l1_block_number(5).await.unwrap();
    database.set_finalized_l1_block_number(1).await.unwrap();

    // create a sequencer
    let config = SequencerConfig {
        chain_spec: node.inner.chain_spec(),
        fee_recipient: Address::random(),
        auto_start: false,
        payload_building_config: PayloadBuildingConfig {
            block_gas_limit: SCROLL_GAS_LIMIT,
            max_l1_messages_per_block: 4,
            l1_message_inclusion_mode: L1MessageInclusionMode::FinalizedWithBlockDepth(0),
        },
        block_time: 0,
        payload_building_duration: 0,
        allow_empty_blocks: true,
    };
    let mut sequencer = Sequencer::new(provider, config);

    // add L1 messages to database
    let wallet_lock = wallet.lock().await;
    let l1_messages = [
        L1MessageEnvelope {
            l1_block_number: 1,
            l2_block_number: None,
            queue_hash: None,
            transaction: TxL1Message {
                queue_index: 0,
                gas_limit: 100_000,
                to: Address::random(),
                value: U256::from(1),
                sender: wallet_lock.inner.address(),
                input: vec![].into(),
            },
        },
        L1MessageEnvelope {
            l1_block_number: 1,
            l2_block_number: None,
            queue_hash: None,
            transaction: TxL1Message {
                queue_index: 1,
                gas_limit: 100_000,
                to: Address::random(),
                value: U256::from(1),
                sender: wallet_lock.inner.address(),
                input: vec![].into(),
            },
        },
        L1MessageEnvelope {
            l1_block_number: 1,
            l2_block_number: None,
            queue_hash: None,
            transaction: TxL1Message {
                queue_index: 2,
                gas_limit: 100_000,
                to: Address::random(),
                value: U256::from(1),
                sender: wallet_lock.inner.address(),
                input: vec![].into(),
            },
        },
        L1MessageEnvelope {
            l1_block_number: 1,
            l2_block_number: None,
            queue_hash: None,
            transaction: TxL1Message {
                queue_index: 3,
                gas_limit: 100_000,
                to: Address::random(),
                value: U256::from(1),
                sender: wallet_lock.inner.address(),
                input: vec![].into(),
            },
        },
    ];
    for l1_message in l1_messages {
        database.insert_l1_message(l1_message).await.unwrap();
    }
    // mark the first two messages as skipped.
    database.update_skipped_l1_messages(vec![0, 1]).await.unwrap();

    // build payload, should only include the last two messages.
    sequencer.start_payload_building(&mut engine).await.unwrap();
    let block = if let SequencerEvent::PayloadReady(payload_id) = sequencer.next().await.unwrap() {
        let block = sequencer.finalize_payload_building(payload_id, &mut engine).await.unwrap();
        assert!(block.is_some(), "expected a new payload, but got: {:?}", block);
        block.unwrap()
    } else {
        panic!("expected a payload ready event");
    };

    // verify only one L1 message is included
    assert_eq!(block.body.transactions.len(), 2);
    assert_eq!(
        block.body.transactions.into_iter().filter_map(|x| x.queue_index()).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(block.header.gas_used, 42_000);
}
