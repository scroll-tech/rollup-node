use metrics::Histogram;
use metrics_derive::Metrics;

/// The metrics for the [`super::Database`].
#[derive(Metrics, Clone)]
#[metrics(scope = "database")]
pub(crate) struct DatabaseMetrics {
    /// Time (ms) to acquire a DB lock/connection
    #[metric(describe = "Time to acquire a database lock (ms)")]
    pub lock_acquire_duration: Histogram,
}
