use alloy_rpc_types_engine::PayloadId;
use reth_scroll_primitives::ScrollBlock;

/// Events emitted by the sequencer.
#[derive(Debug, Clone)]
pub enum SequencerEvent {
    /// A new slot has started.
    NewSlot,
    /// The payload with the given ID is ready to be retrieved from the execution node.
    PayloadReady(PayloadId),
}
