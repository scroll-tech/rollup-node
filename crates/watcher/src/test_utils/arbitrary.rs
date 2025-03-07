use alloy_consensus::{
    Signed, TxEip1559, TxEip2930, TxEip4844, TxEip4844Variant, TxEip7702, TxEnvelope, TxLegacy,
    TxType,
};
use alloy_primitives::{Address, Bytes};
use alloy_rpc_types_eth::Transaction;
use arbitrary::Arbitrary;

/// Returns an arbitrary instance of the passed type.
#[macro_export]
macro_rules! random {
    ($typ: ty) => {{
        let mut bytes = Box::new([0u8; size_of::<$typ>()]);
        let mut rng = ::rand::rng();
        ::rand::RngCore::fill_bytes(&mut rng, bytes.as_mut_slice());
        let mut u = ::arbitrary::Unstructured::new(bytes.as_slice());
        <$typ>::arbitrary(&mut u).unwrap()
    }};
}

/// Helper instance to build an arbitrary transaction.
#[derive(Debug)]
pub struct ArbitraryTxBuilder {
    tx: Transaction,
}

impl ArbitraryTxBuilder {
    /// Creates a random builder for a transaction.
    pub fn new() -> Self {
        let envelope = random!(TxEnvelope);
        Self {
            tx: Transaction {
                inner: envelope,
                block_hash: None,
                block_number: None,
                transaction_index: None,
                effective_gas_price: None,
                from: random!(Address),
            },
        }
    }

    /// Modifies the type of the random transaction.
    pub fn with_ty(mut self, ty: TxType) -> Self {
        match ty {
            TxType::Legacy => self.tx.inner = random!(Signed<TxLegacy>).into(),
            TxType::Eip2930 => self.tx.inner = random!(Signed<TxEip2930>).into(),
            TxType::Eip1559 => self.tx.inner = random!(Signed<TxEip1559>).into(),
            TxType::Eip4844 => self.tx.inner = random!(Signed<TxEip4844>).into(),
            TxType::Eip7702 => self.tx.inner = random!(Signed<TxEip7702>).into(),
        }
        self
    }

    /// Modifies the input of the random transaction.
    pub fn with_input(mut self, input: Bytes) -> Self {
        match self.tx.inner {
            TxEnvelope::Legacy(ref mut tx) => tx.tx_mut().input = input,
            TxEnvelope::Eip2930(ref mut tx) => tx.tx_mut().input = input,
            TxEnvelope::Eip1559(ref mut tx) => tx.tx_mut().input = input,
            TxEnvelope::Eip4844(ref mut tx) => match tx.tx_mut() {
                TxEip4844Variant::TxEip4844(tx) => tx.input = input,
                TxEip4844Variant::TxEip4844WithSidecar(tx) => tx.tx.input = input,
            },
            TxEnvelope::Eip7702(ref mut tx) => tx.tx_mut().input = input,
        }
        self
    }

    /// Returns the built transaction.
    pub fn build(self) -> Transaction {
        self.tx
    }
}
