use reth_network_peers::PeerId;
use scroll_wire::NewBlock;

pub type BlockImportResult = Result<BlockValidation, BlockImportError>;

/// The outcome of a block import operation.
#[derive(Debug)]
pub struct BlockImportOutcome {
    /// The peer that the block was received from.
    pub peer: PeerId,
    /// The result of the block import operation.
    pub result: BlockImportResult,
}

/// The result of a block validation operation.
#[derive(Debug)]
pub enum BlockValidation {
    /// The block header is valid.
    ValidHeader { new_block: NewBlock },
    /// The block is valid.
    ValidBlock { new_block: NewBlock },
}

/// An error that can occur during block import.
#[derive(Debug)]
pub enum BlockImportError {
    /// An error occurred during consensus.
    Consensus(ConsensusError),
    /// An error occurred during block validation.
    Validation(BlockValidationError),
}

/// A consensus related error that can occur during block import.
#[derive(Debug)]
pub enum ConsensusError {
    /// The signature is invalid.
    Signature,
}

#[derive(Debug)]
pub enum BlockValidationError {
    InvalidBlock,
}

impl From<ConsensusError> for BlockImportError {
    fn from(error: ConsensusError) -> Self {
        Self::Consensus(error)
    }
}

impl From<BlockValidationError> for BlockImportError {
    fn from(error: BlockValidationError) -> Self {
        Self::Validation(error)
    }
}
