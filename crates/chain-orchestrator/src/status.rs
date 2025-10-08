use crate::sync::{SyncMode, SyncState};
use scroll_engine::ForkchoiceState;

/// The current status of the chain orchestrator.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChainOrchestratorStatus {
    /// The chain status for L1.
    pub l1: L1ChainStatus,
    /// The chain status for L2.
    pub l2: L2ChainStatus,
}

impl ChainOrchestratorStatus {
    /// Creates a new [`ChainOrchestratorStatus`] from the given sync state, latest L1 block number,
    pub fn new(
        sync_state: &SyncState,
        l1_latest: u64,
        l1_finalized: u64,
        l2_fcs: ForkchoiceState,
    ) -> Self {
        Self {
            l1: L1ChainStatus {
                status: sync_state.l1().clone(),
                latest: l1_latest,
                finalized: l1_finalized,
            },
            l2: L2ChainStatus { status: sync_state.l2().clone(), fcs: l2_fcs },
        }
    }
}

/// The status of the L1 chain.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct L1ChainStatus {
    /// The sync mode of the chain.
    pub status: SyncMode,
    /// The latest block number of the chain.
    pub latest: u64,
    /// The finalized block number of the chain.
    pub finalized: u64,
}

/// The status of the L2 chain.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct L2ChainStatus {
    /// The sync mode of the chain.
    pub status: SyncMode,
    /// The current fork choice state of the chain.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub fcs: ForkchoiceState,
}
