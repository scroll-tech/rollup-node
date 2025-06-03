use metrics::Histogram;
use metrics_derive::Metrics;

/// The metrics for the [`super::Signer`].
#[derive(Metrics, Clone)]
#[metrics(scope = "signer")]
pub struct SignerMetrics {
    /// The signing duration.
    pub signing_duration: Histogram,
}
