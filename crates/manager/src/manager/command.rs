use super::RollupManagerEvent;
use reth_tokio_util::EventStream;
use tokio::sync::oneshot;

/// The commands that can be sent to the rollup manager.
#[derive(Debug)]
pub enum RollupManagerCommand {
    /// Command to build a new block.
    BuildBlock,
    /// Returns an event stream for rollup manager events.
    EventListener(oneshot::Sender<EventStream<RollupManagerEvent>>),
}
