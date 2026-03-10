use super::Header;
use std::time::Instant;

/// A probe that checks L1 liveness by monitoring block timestamps.
#[derive(Debug)]
pub(crate) struct LivenessProbe {
    /// The threshold in seconds after which to log an error if no new block is received.
    threshold: u64,
    /// The interval in seconds at which to perform the liveness check.
    check_interval: u64,
    /// The last time a liveness check was performed.
    last_check: Instant,
}

impl LivenessProbe {
    /// Creates a new liveness probe.
    pub(crate) fn new(threshold: u64, check_interval: u64) -> Self {
        Self { threshold, check_interval, last_check: Instant::now() }
    }

    /// Returns true if a liveness check is due based on the configured interval.
    pub(crate) fn is_due(&self) -> bool {
        self.last_check.elapsed().as_secs() >= self.check_interval
    }

    /// Checks L1 liveness based on the latest block header.
    /// Logs an error if no new block has been received within the threshold.
    pub(crate) fn check(&mut self, latest_block: Option<&Header>) {
        self.last_check = Instant::now();

        if let Some(block) = latest_block {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time went backwards")
                .as_secs();

            let elapsed = now.saturating_sub(block.timestamp);
            if elapsed > self.threshold {
                tracing::error!(
                    target: "scroll::watcher",
                    latest_block_number = block.number,
                    latest_block_timestamp = block.timestamp,
                    elapsed_secs = elapsed,
                    threshold_secs = self.threshold,
                    "L1 liveness check failed: no new L1 block received"
                );
            }
        }
    }
}
