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
}
