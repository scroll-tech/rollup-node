//! Background event streaming for the debug REPL.

use colored::Colorize;
use rollup_node_chain_orchestrator::ChainOrchestratorEvent;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Maximum number of events to keep in history.
const DEFAULT_HISTORY_CAPACITY: usize = 100;

/// State for background event streaming.
#[derive(Debug)]
pub struct EventStreamState {
    /// Whether background streaming is enabled.
    enabled: bool,
    /// Event type filter (glob pattern).
    filter: Option<String>,
    /// Ring buffer of recent events for `events history`.
    history: VecDeque<(Instant, ChainOrchestratorEvent)>,
    /// Max history size.
    history_capacity: usize,
    /// Counter for event numbering.
    event_counter: usize,
}

impl Default for EventStreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl EventStreamState {
    /// Create a new event stream state.
    pub fn new() -> Self {
        Self {
            enabled: false,
            filter: None,
            history: VecDeque::with_capacity(DEFAULT_HISTORY_CAPACITY),
            history_capacity: DEFAULT_HISTORY_CAPACITY,
            event_counter: 0,
        }
    }

    /// Enable background event streaming.
    pub const fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable background event streaming.
    pub const fn disable(&mut self) {
        self.enabled = false;
    }

    /// Check if streaming is enabled.
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Set the event filter pattern.
    pub fn set_filter(&mut self, pattern: Option<String>) {
        self.filter = pattern;
    }

    /// Get the current filter pattern.
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// Record an event in history and optionally display it.
    pub fn record_event(&mut self, event: ChainOrchestratorEvent) -> Option<String> {
        let now = Instant::now();

        // Add to history
        if self.history.len() >= self.history_capacity {
            self.history.pop_front();
        }
        self.history.push_back((now, event.clone()));
        self.event_counter += 1;

        // Check if we should display this event
        if !self.enabled {
            return None;
        }

        let event_name = event_type_name(&event);
        if !self.matches_filter(&event_name) {
            return None;
        }

        Some(self.format_event(&event))
    }

    /// Check if an event name matches the filter.
    fn matches_filter(&self, event_name: &str) -> bool {
        match &self.filter {
            None => true,
            Some(pattern) => {
                // Simple glob matching: * matches any sequence of characters
                let pattern = pattern.replace('*', ".*");
                regex_lite::Regex::new(&format!("^{}$", pattern))
                    .map(|re| re.is_match(event_name))
                    .unwrap_or(true)
            }
        }
    }

    /// Format an event for display.
    pub fn format_event(&self, event: &ChainOrchestratorEvent) -> String {
        let prefix = "  [EVENT]".cyan();
        let event_str = format_event_short(event);
        format!("{} {}", prefix, event_str)
    }

    /// Get recent events from history.
    pub fn get_history(&self, count: usize) -> Vec<(Duration, &ChainOrchestratorEvent)> {
        let start = Instant::now();
        self.history
            .iter()
            .rev()
            .take(count)
            .map(|(t, e)| (start.duration_since(*t), e))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// Get the total number of events recorded.
    pub const fn total_events(&self) -> usize {
        self.event_counter
    }
}

/// Get the type name of an event for filtering.
pub fn event_type_name(event: &ChainOrchestratorEvent) -> String {
    match event {
        ChainOrchestratorEvent::BlockSequenced(_) => "BlockSequenced".to_string(),
        ChainOrchestratorEvent::ChainConsolidated { .. } => "ChainConsolidated".to_string(),
        ChainOrchestratorEvent::ChainExtended(_) => "ChainExtended".to_string(),
        ChainOrchestratorEvent::ChainReorged(_) => "ChainReorged".to_string(),
        ChainOrchestratorEvent::L1Synced => "L1Synced".to_string(),
        ChainOrchestratorEvent::OptimisticSync(_) => "OptimisticSync".to_string(),
        ChainOrchestratorEvent::NewL1Block(_) => "NewL1Block".to_string(),
        ChainOrchestratorEvent::L1MessageCommitted(_) => "L1MessageCommitted".to_string(),
        ChainOrchestratorEvent::L1Reorg { .. } => "L1Reorg".to_string(),
        ChainOrchestratorEvent::BatchConsolidated(_) => "BatchConsolidated".to_string(),
        ChainOrchestratorEvent::UnwoundToL1Block(_) => "UnwoundToL1Block".to_string(),
        ChainOrchestratorEvent::BlockConsolidated(_) => "BlockConsolidated".to_string(),
        ChainOrchestratorEvent::BatchReverted { .. } => "BatchReverted".to_string(),
        ChainOrchestratorEvent::L1BlockFinalized(_, _) => "L1BlockFinalized".to_string(),
        ChainOrchestratorEvent::NewBlockReceived(_) => "NewBlockReceived".to_string(),
        ChainOrchestratorEvent::L1MessageNotFoundInDatabase(_) => {
            "L1MessageNotFoundInDatabase".to_string()
        }
        ChainOrchestratorEvent::BlockFailedConsensusChecks(_, _) => {
            "BlockFailedConsensusChecks".to_string()
        }
        ChainOrchestratorEvent::InsufficientDataForReceivedBlock(_) => {
            "InsufficientDataForReceivedBlock".to_string()
        }
        ChainOrchestratorEvent::BlockAlreadyKnown(_, _) => "BlockAlreadyKnown".to_string(),
        ChainOrchestratorEvent::OldForkReceived { .. } => "OldForkReceived".to_string(),
        ChainOrchestratorEvent::BatchCommitIndexed { .. } => "BatchCommitIndexed".to_string(),
        ChainOrchestratorEvent::BatchFinalized { .. } => "BatchFinalized".to_string(),
        ChainOrchestratorEvent::L2ChainCommitted(_, _, _) => "L2ChainCommitted".to_string(),
        ChainOrchestratorEvent::L2ConsolidatedBlockCommitted(_) => {
            "L2ConsolidatedBlockCommitted".to_string()
        }
        ChainOrchestratorEvent::SignedBlock { .. } => "SignedBlock".to_string(),
        ChainOrchestratorEvent::L1MessageMismatch { .. } => "L1MessageMismatch".to_string(),
        ChainOrchestratorEvent::FcsHeadUpdated(_) => "FcsHeadUpdated".to_string(),
    }
}

/// Format an event for short display.
pub fn format_event_short(event: &ChainOrchestratorEvent) -> String {
    match event {
        ChainOrchestratorEvent::BlockSequenced(block) => {
            format!(
                "BlockSequenced {{ block: {}, hash: {:.8}... }}",
                block.header.number,
                format!("{:?}", block.header.hash_slow())
            )
        }
        ChainOrchestratorEvent::ChainConsolidated { from, to } => {
            format!("ChainConsolidated {{ from: {}, to: {} }}", from, to)
        }
        ChainOrchestratorEvent::ChainExtended(import) => {
            format!("ChainExtended {{ blocks: {} }}", import.chain.len())
        }
        ChainOrchestratorEvent::ChainReorged(import) => {
            format!("ChainReorged {{ blocks: {} }}", import.chain.len())
        }
        ChainOrchestratorEvent::L1Synced => "L1Synced".to_string(),
        ChainOrchestratorEvent::OptimisticSync(info) => {
            format!("OptimisticSync {{ block: {} }}", info.number)
        }
        ChainOrchestratorEvent::NewL1Block(num) => format!("NewL1Block {{ block: {} }}", num),
        ChainOrchestratorEvent::L1MessageCommitted(queue_index) => {
            format!("L1MessageCommitted {{ queue_index: {} }}", queue_index)
        }
        ChainOrchestratorEvent::L1Reorg { l1_block_number, .. } => {
            format!("L1Reorg {{ l1_block: {} }}", l1_block_number)
        }
        ChainOrchestratorEvent::BatchConsolidated(outcome) => {
            format!("BatchConsolidated {{ blocks: {} }}", outcome.blocks.len())
        }
        ChainOrchestratorEvent::UnwoundToL1Block(num) => {
            format!("UnwoundToL1Block {{ block: {} }}", num)
        }
        ChainOrchestratorEvent::BlockConsolidated(outcome) => {
            format!("BlockConsolidated {{ block: {} }}", outcome.block_info().block_info.number)
        }
        ChainOrchestratorEvent::BatchReverted { batch_info, safe_head } => {
            format!("BatchReverted {{ index: {}, safe: {} }}", batch_info.index, safe_head.number)
        }
        ChainOrchestratorEvent::L1BlockFinalized(num, batches) => {
            format!("L1BlockFinalized {{ block: {}, batches: {} }}", num, batches.len())
        }
        ChainOrchestratorEvent::NewBlockReceived(nbwp) => {
            format!(
                "NewBlockReceived {{ block: {}, peer: {:.8}... }}",
                nbwp.block.header.number,
                format!("{:?}", nbwp.peer_id)
            )
        }
        ChainOrchestratorEvent::L1MessageNotFoundInDatabase(key) => {
            format!("L1MessageNotFoundInDatabase {{ key: {:?} }}", key)
        }
        ChainOrchestratorEvent::BlockFailedConsensusChecks(hash, peer) => {
            format!(
                "BlockFailedConsensusChecks {{ hash: {:.8}..., peer: {:.8}... }}",
                format!("{:?}", hash),
                format!("{:?}", peer)
            )
        }
        ChainOrchestratorEvent::InsufficientDataForReceivedBlock(hash) => {
            format!("InsufficientDataForReceivedBlock {{ hash: {:.8}... }}", format!("{:?}", hash))
        }
        ChainOrchestratorEvent::BlockAlreadyKnown(hash, peer) => {
            format!(
                "BlockAlreadyKnown {{ hash: {:.8}..., peer: {:.8}... }}",
                format!("{:?}", hash),
                format!("{:?}", peer)
            )
        }
        ChainOrchestratorEvent::OldForkReceived { headers, peer_id, .. } => {
            format!(
                "OldForkReceived {{ headers: {}, peer: {:.8}... }}",
                headers.len(),
                format!("{:?}", peer_id)
            )
        }
        ChainOrchestratorEvent::BatchCommitIndexed { batch_info, l1_block_number } => {
            format!(
                "BatchCommitIndexed {{ index: {}, l1_block: {} }}",
                batch_info.index, l1_block_number
            )
        }
        ChainOrchestratorEvent::BatchFinalized { l1_block_info, triggered_batches } => {
            format!(
                "BatchFinalized {{ l1_block: {}, batches: {} }}",
                l1_block_info.number,
                triggered_batches.len()
            )
        }
        ChainOrchestratorEvent::L2ChainCommitted(info, batch, is_consolidated) => {
            format!(
                "L2ChainCommitted {{ block: {}, batch: {:?}, consolidated: {} }}",
                info.block_info.number,
                batch.as_ref().map(|b| b.index),
                is_consolidated
            )
        }
        ChainOrchestratorEvent::L2ConsolidatedBlockCommitted(info) => {
            format!("L2ConsolidatedBlockCommitted {{ block: {} }}", info.block_info.number)
        }
        ChainOrchestratorEvent::SignedBlock { block, .. } => {
            format!(
                "SignedBlock {{ block: {}, hash: {:.8}... }}",
                block.header.number,
                format!("{:?}", block.header.hash_slow())
            )
        }
        ChainOrchestratorEvent::L1MessageMismatch { expected, actual } => {
            format!(
                "L1MessageMismatch {{ expected: {:.8}..., actual: {:.8}... }}",
                format!("{:?}", expected),
                format!("{:?}", actual)
            )
        }
        ChainOrchestratorEvent::FcsHeadUpdated(info) => {
            format!("FcsHeadUpdated {{ block: {} }}", info.number)
        }
    }
}
