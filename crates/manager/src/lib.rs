//! This library contains the main manager for the rollup node.

pub use consensus::{Consensus, NoopConsensus, SystemContractConsensus};
mod consensus;

mod manager;
pub use manager::{
    RollupManagerCommand, RollupManagerEvent, RollupManagerHandle, RollupNodeManager,
};
