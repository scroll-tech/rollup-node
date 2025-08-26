use metrics::{Counter, Gauge};
use metrics_derive::Metrics;

/// The metrics for the [`super::DerivationPipeline`].
#[derive(Metrics, Clone)]
#[metrics(scope = "derivation_pipeline")]
pub struct DerivationPipelineMetrics {
    /// A counter on the derived L2 blocks processed.
    pub derived_blocks: Counter,
    /// The blocks per second derived by the pipeline.
    pub blocks_per_second: Gauge,
    /// The number of batches in the queue.
    pub batch_queue_size: Gauge,
    /// The number of payload attributes in the queue.
    pub payload_attributes_queue_size: Gauge,
}
