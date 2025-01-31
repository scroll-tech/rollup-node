use reth_scroll_primitives::ScrollBlock;
use scroll_engine::EngineDriver;
use scroll_network::NetworkHandle;

pub struct RollupNodeManager<C, EC, P> {
    network: NetworkHandle,
    engine: EngineDriver<EC, P>,
    consensus: C,
}

trait Consensus {
    fn validate_new_block(&self, block: ScrollBlock) -> Result<(), ConsensusError>;
}

/// A consensus related error that can occur during block import.
pub enum ConsensusError {
    /// The block is invalid.
    Block,
    /// The state root is invalid.
    StateRoot,
    /// The signature is invalid.
    Signature,
}
