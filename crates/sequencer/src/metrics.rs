use metrics::Histogram;
use metrics_derive::Metrics;

/// The metrics for the [`super::Sequencer`].
#[derive(Metrics, Clone)]
#[metrics(scope = "sequencer")]
pub struct SequencerMetrics {
    /// The payload attributes building duration.
    pub payload_attributes_building_duration: Histogram,
}
