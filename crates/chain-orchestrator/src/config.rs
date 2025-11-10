use rollup_node_primitives::BlockInfo;
use std::sync::Arc;

/// Configuration for the chain orchestrator.
#[derive(Debug)]
pub struct ChainOrchestratorConfig<ChainSpec> {
    /// The chain specification.
    chain_spec: Arc<ChainSpec>,
    /// The threshold for optimistic sync. If the received block is more than this many blocks
    /// ahead of the current chain, we optimistically sync the chain.
    optimistic_sync_threshold: u64,
    /// The L1 message queue index at which the V2 L1 message queue was enabled.
    l1_v2_message_queue_start_index: u64,
    /// The forkchoice target.
    forkchoice_state_target: Option<BlockInfo>,
}

impl<ChainSpec> ChainOrchestratorConfig<ChainSpec> {
    /// Creates a new chain configuration.
    pub const fn new(
        chain_spec: Arc<ChainSpec>,
        optimistic_sync_threshold: u64,
        l1_v2_message_queue_start_index: u64,
        forkchoice_state_target: Option<BlockInfo>,
    ) -> Self {
        Self {
            chain_spec,
            optimistic_sync_threshold,
            l1_v2_message_queue_start_index,
            forkchoice_state_target,
        }
    }

    /// Returns a reference to the chain specification.
    pub const fn chain_spec(&self) -> &Arc<ChainSpec> {
        &self.chain_spec
    }

    /// Returns the optimistic sync threshold.
    pub const fn optimistic_sync_threshold(&self) -> u64 {
        self.optimistic_sync_threshold
    }

    /// Returns the L1 message queue index at which the V2 L1 message queue was enabled.
    pub const fn l1_v2_message_queue_start_index(&self) -> u64 {
        self.l1_v2_message_queue_start_index
    }

    /// Returns the forkchoice state target.
    pub const fn forkchoice_state_target(&self) -> Option<&BlockInfo> {
        self.forkchoice_state_target.as_ref()
    }
}
