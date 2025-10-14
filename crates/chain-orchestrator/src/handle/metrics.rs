use metrics::Counter;
use metrics_derive::Metrics;

/// The metrics for the [`super::ChainOrchestratorHandle`].
#[derive(Metrics, Clone)]
#[metrics(scope = "NodeManager")]
pub(crate) struct ChainOrchestratorHandleMetrics {
    /// Failed to send command to rollup manager from handle counter.
    pub handle_send_command_failed: Counter,
}
