//! This library contains the main manager for the rollup node.

pub use consensus::{Consensus, NoopConsensus, SystemContractConsensus};
mod consensus;

mod manager;
pub use manager::{
    compute_watcher_start_block_from_database, RollupManagerCommand, RollupManagerEvent,
    RollupManagerHandle, RollupNodeManager,
};
