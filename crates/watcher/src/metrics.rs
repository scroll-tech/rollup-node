use crate::L1Notification;

use metrics::{Counter, Histogram};
use metrics_derive::Metrics;

/// The metrics for the [`super::L1Watcher`].
#[derive(Metrics)]
#[metrics(scope = "l1_watcher")]
pub struct WatcherMetrics {
    /// A counter on the L1 messages processed.
    pub l1_messages: Counter,
    /// A counter on the batch commits processed.
    pub batch_commits: Counter,
    /// A counter on the batch finalization processed.
    pub batch_finalizations: Counter,
    /// A counter on the reorgs.
    pub reorgs: Counter,
    /// A histogram of reorgs depth.
    pub reorg_depths: Histogram,
}

impl WatcherMetrics {
    /// Processed an L1 notification by updating the appropriate metric.
    pub fn process_l1_notification(&self, notification: &L1Notification) {
        match notification {
            L1Notification::L1Message { .. } => self.l1_messages.increment(1),
            L1Notification::BatchCommit(_) => self.batch_commits.increment(1),
            L1Notification::BatchFinalization { .. } => self.batch_finalizations.increment(1),
            _ => {}
        }
    }
}
