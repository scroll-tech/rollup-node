//! Integration tests for L1 data fee buffer behavior in the rollup node txpool.

use std::str::FromStr;

use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{private::rand::random_iter, Address, Bytes, TxKind, B256, U256};
use alloy_rpc_types_eth::{TransactionInput, TransactionRequest};
use reth_chainspec::EthChainSpec;
use reth_rpc_api::EthApiServer;
use reth_rpc_eth_api::SignableTxRequest;
use reth_scroll_chainspec::{ScrollChainConfig, ScrollChainSpecBuilder, SCROLL_DEV};
use rollup_node::test_utils::TestFixture;
use scroll_alloy_evm::gas_price_oracle::L1_GAS_PRICE_ORACLE_ADDRESS;
use scroll_alloy_rpc_types::ScrollTransactionRequest;
use std::sync::Arc;

#[tokio::test]
async fn test_l1_data_fee_buffer() -> eyre::Result<()> {
    reth_tracing::init_test_tracing();

    let mut genesis = SCROLL_DEV.genesis.clone();

    // Fund the genesis signer with a balance that will be:
    // - Sufficient for 1x L1 data fee
    // - Insufficient for 2x L1 data fee
    let signer = Address::from_str("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266")?;
    let balance = U256::from(800_000_000_000_000u128);
    genesis.alloc.get_mut(&signer).expect("no genesis signer found").balance = balance;

    // Set the L1 Gas Price Oracle storage slots to create significant L1 data fees.
    let l1_gas_price_oracle_storage = genesis
        .alloc
        .get_mut(&L1_GAS_PRICE_ORACLE_ADDRESS)
        .expect("no L1 Gas Price Oracle account found in genesis")
        .storage
        .as_mut()
        .expect("storage not found for L1 gas price oracle");
    for slot in 0u8..8 {
        let key = B256::from(U256::from(slot));
        *l1_gas_price_oracle_storage.entry(key).or_default() = U256::from(u32::MAX).into();
    }

    // Build the chain spec with the modified genesis.
    let chain_spec = Arc::new(
        ScrollChainSpecBuilder::default()
            .chain(SCROLL_DEV.chain())
            .genesis(genesis)
            .with_forks(SCROLL_DEV.hardforks.clone())
            .build(ScrollChainConfig::dev()),
    );

    // Define the base test fixture.
    let base_fixture =
        TestFixture::builder().sequencer().with_chain_spec(chain_spec.clone()).config(|cfg| {
            cfg.sequencer_args.auto_start = true;
            cfg.sequencer_args.allow_empty_blocks = false;
        });

    // Instantiate the test fixture without L1 data fee buffer requirement.
    let mut fixture_no_buffer = base_fixture
        .clone()
        .config(|cfg| {
            cfg.require_l1_data_fee_buffer = false;
        })
        .build()
        .await?;
    fixture_no_buffer.l1().sync().await?;

    // Construct the test transaction
    let signed_tx: Bytes = {
        let wallet = fixture_no_buffer.wallet.lock().await;
        let gas_price = 19_000_000u128;
        let gas_limit = 1_000_000u64;
        // Use large transaction data to create significant L1 data fees
        let tx_input_bytes = Bytes::from(random_iter::<u8>().take(19_000).collect::<Vec<u8>>());
        let tx_req = TransactionRequest {
            nonce: Some(0),
            to: Some(TxKind::Call(Address::random())),
            gas: Some(gas_limit),
            gas_price: Some(gas_price),
            chain_id: Some(chain_spec.chain_id()),
            input: TransactionInput { input: None, data: Some(tx_input_bytes.clone()) },
            ..Default::default()
        };
        let scroll_tx_request: ScrollTransactionRequest = tx_req.into();
        let signed_tx = scroll_tx_request.clone().try_build_and_sign(wallet.inner.clone()).await?;
        signed_tx.encoded_2718().into()
    };

    // Submit the transaction without L1 data fee buffer requirement
    let eth_api = fixture_no_buffer.sequencer().node.rpc.inner.eth_api();
    if eth_api.send_raw_transaction_sync(signed_tx.clone()).await.is_err() {
        eyre::bail!("Transaction should not be rejected without L1 data fee buffer requirement");
    }
    drop(fixture_no_buffer);

    // Instantiate the test fixture with L1 data fee buffer requirement.
    let mut fixture_with_buffer = base_fixture
        .config(|cfg| {
            cfg.require_l1_data_fee_buffer = true;
        })
        .build()
        .await?;
    fixture_with_buffer.l1().sync().await?;

    // Submit the transaction with L1 data fee buffer requirement
    let eth_api = fixture_with_buffer.sequencer().node.rpc.inner.eth_api();
    if eth_api.send_raw_transaction_sync(signed_tx).await.is_ok() {
        eyre::bail!("Transaction should be rejected with L1 data fee buffer requirement");
    }
    drop(fixture_with_buffer);

    Ok(())
}
