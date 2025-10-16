use metrics::Histogram;
use metrics_derive::Metrics;
use std::{collections::HashMap, time::Instant};
use strum::{EnumIter, IntoEnumIterator};

/// The metric handler for the chain orchestrator. Tracks execution duration of various tasks.
#[derive(Debug)]
pub struct MetricsHandler {
    /// The chain orchestrator metrics.
    chain_orchestrator_tasks_metrics: HashMap<Task, ChainOrchestratorMetrics>,
    /// The inflight block building meter.
    block_building_meter: BlockBuildingMeter,
}

impl MetricsHandler {
    /// Returns the [`ChainOrchestratorMetrics`] for the provided task.
    pub fn get(&self, task: Task) -> Option<&ChainOrchestratorMetrics> {
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
    pub(crate) fn finish_block_building_recording(&mut self) {
        let duration = self.block_building_meter.start.take().map(|start| start.elapsed());
        if let Some(duration) = duration {
            self.block_building_meter.metric.block_building_duration.record(duration.as_secs_f64());
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

/// An enum representing the chain orchestrator tasks
#[derive(Debug, PartialEq, Eq, Hash, EnumIter)]
pub enum Task {
    /// Batch reconciliation with the unsafe L2 chain.
    BatchReconciliation,
    /// L1 consolidation.
    L1Consolidation,
    /// L1 reorg.
    L1Reorg,
    /// L1 finalization.
    L1Finalization,
    /// L1 message.
    L1Message,
    /// Batch commit.
    BatchCommit,
    /// Batch finalization.
    BatchFinalization,
}

impl Task {
    /// Returns the str representation of the [`ChainOrchestratorItem`].
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::L1Reorg => "l1_reorg",
            Self::L1Finalization => "l1_finalization",
            Self::L1Message => "l1_message",
            Self::BatchCommit => "batch_commit",
            Self::BatchFinalization => "batch_finalization",
            Self::BatchReconciliation => "batch_reconciliation",
            Self::L1Consolidation => "l1_consolidation",
        }
    }
}

/// The metrics for the [`super::ChainOrchestrator`].
#[derive(Metrics, Clone)]
#[metrics(scope = "chain_orchestrator")]
pub struct ChainOrchestratorMetrics {
    /// The duration of the task for the chain orchestrator.
    pub task_duration: Histogram,
}

/// Block building related metric.

#[derive(Metrics, Clone)]
#[metrics(scope = "chain_orchestrator")]
pub(crate) struct BlockBuildingMetric {
    /// The duration of the block building task.
    block_building_duration: Histogram,
}

/// A block building meter.
#[derive(Debug, Default)]
pub(crate) struct BlockBuildingMeter {
    metric: BlockBuildingMetric,
    start: Option<Instant>,
}
