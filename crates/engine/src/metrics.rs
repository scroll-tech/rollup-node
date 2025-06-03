use metrics::Histogram;
use metrics_derive::Metrics;

/// The metrics for the [`super::EngineDriver`].
#[derive(Metrics, Clone)]
#[metrics(scope = "engine_driver")]
pub struct EngineDriverMetrics {
    /// The duration for a new payload to be built.
    pub build_new_payload_duration: Histogram,
    /// The current throughput of the driver in gas/block.
    pub gas_per_block: Histogram,
    /// The duration for a l1 consolidation.
    pub l1_consolidation_duration: Histogram,
    /// The duration for a block import.
    pub block_import_duration: Histogram,
}
