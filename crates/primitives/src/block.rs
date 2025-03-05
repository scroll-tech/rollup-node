use alloy_consensus::Header;
use alloy_primitives::B256;
use reth_scroll_primitives::ScrollTransactionSigned;

/// A [`BlockCommitment`] is a single block commitment that is settled to L1.
#[derive(Debug)]
pub struct BlockContext {
    /// The header of the block commitment.
    pub header: Header,
    /// The transactions in the block commitment.
    pub transactions: Vec<ScrollTransactionSigned>,
    /// The withdrawal root of the block commitment.
    pub withdraw_root: B256,
    /// The prover row consumption of the block commitment.
    pub row_consumption: RowConsumption,
}

/// The row consumption of a block commitment.
#[derive(Debug)]
pub struct RowConsumption(pub Vec<SubCircuitRowConsumption>);

/// The row consumption of a sub-circuit.
#[derive(Debug)]
pub struct SubCircuitRowConsumption {
    /// The name of the sub-circuit.
    pub name: String,
    /// The number of rows consumed by the sub-circuit.
    pub row_number: u64,
}
