use metrics::{Counter, Gauge};
use metrics_derive::Metrics;

/// The metrics for the [`super::RollupManagerHandle`].
#[derive(Metrics, Clone)]
#[metrics(scope = "NodeManager")]
pub(crate) struct HandleMetrics {
    /// Failed to send command to rollup manager from handle counter.
    pub handle_send_command_failed: Counter,
}

/// The metrics for the [`super::RollupNodeManager`].
#[derive(Metrics, Clone)]
#[metrics(scope = "NodeManager")]
pub(crate) struct RollupNodeManagerMetrics {
    /// Manager received and handle rollup manager command counter.
    pub handle_rollup_manager_command: Counter,
    /// Manager received and handle engine driver event counter.
    pub handle_engine_driver_event: Counter,
    /// Manager received and handle new block produced counter.
    pub handle_new_block_produced: Counter,
    /// Manager received and handle l1 notification counter.
    pub handle_l1_notification: Counter,
    /// Manager received and handle chain orchestrator event counter.
    pub handle_chain_orchestrator_event: Counter,
    /// Manager received and handle signer event counter.
    pub handle_signer_event: Counter,
    /// Manager received and handle build new payload counter.
    pub handle_build_new_payload: Counter,
    /// Manager received and handle l1 consolidation counter.
    pub handle_l1_consolidation: Counter,
    /// Manager received and handle network manager event counter.
    pub handle_network_manager_event: Counter,
    /// Manager finalized batch index gauge.
    pub handle_finalized_batch_index: Gauge,
    /// Manager l1 finalized block number gauge.
    pub handle_l1_finalized_block_number: Gauge,
    /// Manager L1 reorg L1 block number gauge.
    pub handle_l1_reorg_l1_block_number: Gauge,
    /// Manager L1 reorg L2 head block number gauge.
    pub handle_l1_reorg_l2_head_block_number: Gauge,
    /// Manager L1 reorg L2 safe block number gauge.
    pub handle_l1_reorg_l2_safe_block_number: Gauge,
    /// Manager chain import block number gauge.
    pub handle_chain_import_block_number: Gauge,
    /// Manager optimistic syncing block number gauge.
    pub handle_optimistic_syncing_block_number: Gauge,
}
