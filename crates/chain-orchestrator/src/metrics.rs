use metrics::{Counter, Histogram};
use metrics_derive::Metrics;
use std::{collections::HashMap, time::Instant};
use strum::{EnumIter, IntoEnumIterator};

/// The metric handler for the chain orchestrator. Tracks execution duration of various tasks.
#[derive(Debug)]
pub(crate) struct MetricsHandler {
    /// The chain orchestrator metrics.
    chain_orchestrator_tasks_metrics: HashMap<Task, ChainOrchestratorMetrics>,
    /// The inflight block building meter.
    block_building_meter: BlockBuildingMeter,
}

impl MetricsHandler {
    /// Returns the [`ChainOrchestratorMetrics`] for the provided task.
    pub(crate) fn get(&self, task: Task) -> Option<&ChainOrchestratorMetrics> {
        self.chain_orchestrator_tasks_metrics.get(&task)
    }

    /// Starts tracking a new block building task.
    pub(crate) fn start_block_building_recording(&mut self) {
        if self.block_building_meter.start.is_some() {
            tracing::warn!(target: "scroll::chain_orchestrator", "block building recording is already ongoing, overwriting");
        }
        self.block_building_meter.start = Some(Instant::now());
    }

    /// The duration of the current block building task if any.
    pub(crate) fn finish_block_building_recording(&mut self, gas_used: u64) {
        // Always increment the cumulative gas counter (similar to geth's commitGasCounter)
        self.block_building_meter.metric.commit_gas.increment(gas_used);

        let duration = self.block_building_meter.start.take().map(|start| start.elapsed());
        if let Some(duration) = duration {
            let duration_secs = duration.as_secs_f64();
            self.block_building_meter.metric.block_building_duration.record(duration_secs);
            self.block_building_meter.metric.gas_per_block.record(gas_used as f64);
            if duration_secs > 0.0 {
                self.block_building_meter
                    .metric
                    .gas_per_second
                    .record(gas_used as f64 / duration_secs);
            }
        }
    }
}

impl Default for MetricsHandler {
    fn default() -> Self {
        Self {
            chain_orchestrator_tasks_metrics: Task::iter()
                .map(|i| {
                    let label = i.as_str();
                    (i, ChainOrchestratorMetrics::new_with_labels(&[("task", label)]))
                })
                .collect(),
            block_building_meter: BlockBuildingMeter::default(),
        }
    }
}

/// An enum representing the chain orchestrator tasks.
#[derive(Debug, PartialEq, Eq, Hash, EnumIter)]
pub(crate) enum Task {
    /// Batch reconciliation with the unsafe L2 chain.
    BatchReconciliation,
    /// Import of an L2 block received over p2p.
    L2BlockImport,
    /// Consolidation of the L2 ledger by validating unsafe blocks.
    ChainConsolidation,
    /// L1 reorg handling.
    L1Reorg,
    /// L1 finalization handling.
    L1Finalization,
    /// L1 message handling.
    L1Message,
    /// Batch commit event handling.
    BatchCommit,
    /// Batch finalization event handling.
    BatchFinalization,
    /// Batch revert event handling.
    BatchRevert,
    /// Batch revert range event handling.
    BatchRevertRange,
}

impl Task {
    /// Returns the str representation of the [`ChainOrchestratorItem`].
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::L1Reorg => "l1_reorg",
            Self::L1Finalization => "l1_finalization",
            Self::L1Message => "l1_message",
            Self::BatchCommit => "batch_commit",
            Self::BatchFinalization => "batch_finalization",
            Self::BatchRevert => "batch_revert",
            Self::BatchRevertRange => "batch_revert_range",
            Self::BatchReconciliation => "batch_reconciliation",
            Self::ChainConsolidation => "chain_consolidation",
            Self::L2BlockImport => "l2_block_import",
        }
    }
}

/// The metrics for the [`super::ChainOrchestrator`].
#[derive(Metrics, Clone)]
#[metrics(scope = "chain_orchestrator")]
pub(crate) struct ChainOrchestratorMetrics {
    /// The duration of the task for the chain orchestrator.
    pub task_duration: Histogram,
}

/// A block building meter.
#[derive(Debug, Default)]
pub(crate) struct BlockBuildingMeter {
    metric: BlockBuildingMetric,
    start: Option<Instant>,
}

/// Block building related metric.
#[derive(Metrics, Clone)]
#[metrics(scope = "chain_orchestrator")]
pub(crate) struct BlockBuildingMetric {
    /// The duration of the block building task.
    block_building_duration: Histogram,
    /// The gas used per block.
    pub(crate) gas_per_block: Histogram,
    /// The gas throughput in gas/second.
    pub(crate) gas_per_second: Histogram,
    /// The cumulative gas used across all committed blocks (similar to geth's `miner/commit_gas`).
    pub(crate) commit_gas: Counter,
}
