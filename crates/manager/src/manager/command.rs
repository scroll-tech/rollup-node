use super::{RollupManagerEvent, RollupManagerStatus};

use reth_tokio_util::EventStream;
use rollup_node_primitives::BlockInfo;
use tokio::sync::oneshot;

/// The commands that can be sent to the rollup manager.
#[derive(Debug)]
pub enum RollupManagerCommand {
    /// Command to build a new block.
    BuildBlock,
    /// Returns an event stream for rollup manager events.
    EventListener(oneshot::Sender<EventStream<RollupManagerEvent>>),
    /// Report the current status of the manager via the oneshot channel.
    Status(oneshot::Sender<RollupManagerStatus>),
    /// Update the head of the fcs in the engine driver.
    UpdateFcsHead(BlockInfo),
}
