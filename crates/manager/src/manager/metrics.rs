use metrics::Counter;
use metrics_derive::Metrics;

/// The metrics for the [`Manager`].
#[derive(Metrics, Clone)]
#[metrics(scope = "Manager")]
pub(crate) struct HandleMetrics {
    /// Failed to send command to rollup manager from handle counter.
    pub handle_send_command_failed: Counter,
}

