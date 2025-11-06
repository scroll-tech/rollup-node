//! Transaction creation and injection helpers for test fixtures.

use super::fixture::TestFixture;
use alloy_primitives::{Address, Bytes, B256, U256};
use reth_e2e_test_utils::transaction::TransactionTestContext;

/// Helper for creating and injecting transactions in tests.
#[derive(Debug)]
pub struct TxHelper<'a> {
    fixture: &'a mut TestFixture,
    target_node_index: usize,
}

impl<'a> TxHelper<'a> {
    /// Create a new transaction helper.
    pub(crate) fn new(fixture: &'a mut TestFixture) -> Self {
        Self { fixture, target_node_index: 0 }
    }

    /// Target a specific node for transaction injection (default is sequencer).
    pub const fn for_node(mut self, index: usize) -> Self {
        self.target_node_index = index;
        self
    }

    /// Create a transfer transaction builder.
    pub fn transfer(self) -> TransferTxBuilder<'a> {
        TransferTxBuilder::new(self)
    }
}

/// Builder for creating transfer transactions.
#[derive(Debug)]
pub struct TransferTxBuilder<'a> {
    tx_helper: TxHelper<'a>,
    to: Option<Address>,
    value: U256,
}

impl<'a> TransferTxBuilder<'a> {
    /// Create a new transfer transaction builder.
    fn new(tx_helper: TxHelper<'a>) -> Self {
        Self { tx_helper, to: None, value: U256::from(1) }
    }

    /// Set the recipient address.
    pub const fn to(mut self, address: Address) -> Self {
        self.to = Some(address);
        self
    }

    /// Set the value to transfer.
    pub fn value(mut self, value: impl Into<U256>) -> Self {
        self.value = value.into();
        self
    }

    /// Build and inject the transaction, returning its hash.
    pub async fn inject(self) -> eyre::Result<B256> {
        let mut wallet = self.tx_helper.fixture.wallet.lock().await;

        // Generate the transaction
        let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
            wallet.chain_id,
            wallet.inner.clone(),
            wallet.inner_nonce,
        )
        .await;

        wallet.inner_nonce += 1;
        drop(wallet);

        // Inject into the target node
        let node = &self.tx_helper.fixture.nodes[self.tx_helper.target_node_index];
        let tx_hash = node.node.rpc.inject_tx(raw_tx).await?;

        Ok(tx_hash)
    }

    /// Build the transaction bytes without injecting.
    pub async fn build(self) -> eyre::Result<Bytes> {
        let mut wallet = self.tx_helper.fixture.wallet.lock().await;

        let raw_tx = TransactionTestContext::transfer_tx_nonce_bytes(
            wallet.chain_id,
            wallet.inner.clone(),
            wallet.inner_nonce,
        )
        .await;

        wallet.inner_nonce += 1;

        Ok(raw_tx)
    }
}
