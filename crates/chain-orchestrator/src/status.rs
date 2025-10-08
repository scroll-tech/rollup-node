use super::SyncState;
use scroll_engine::ForkchoiceState;

/// The current status of the chain orchestrator.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChainOrchestratorStatus {
    /// The current sync state of the orchestrator.
    pub sync_state: SyncState,
    /// The current FCS for the manager.
    pub forkchoice_state: ForkchoiceState,
}
