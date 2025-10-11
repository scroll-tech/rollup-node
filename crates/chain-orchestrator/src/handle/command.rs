use crate::{ChainOrchestratorEvent, ChainOrchestratorStatus};

use reth_network_api::FullNetwork;
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_tokio_util::EventStream;
use rollup_node_primitives::BlockInfo;
use scroll_network::ScrollNetworkHandle;
use tokio::sync::oneshot;

/// The commands that can be sent to the rollup manager.
#[derive(Debug)]
pub enum ChainOrchestratorCommand<N: FullNetwork<Primitives = ScrollNetworkPrimitives>> {
    /// Command to build a new block.
    BuildBlock,
    /// Returns an event stream for rollup manager events.
    EventListener(oneshot::Sender<EventStream<ChainOrchestratorEvent>>),
    /// Report the current status of the manager via the oneshot channel.
    Status(oneshot::Sender<ChainOrchestratorStatus>),
    /// Returns the network handle.
    NetworkHandle(oneshot::Sender<ScrollNetworkHandle<N>>),
    /// Update the head of the fcs in the engine driver.
    UpdateFcsHead((BlockInfo, oneshot::Sender<()>)),
    /// Enable automatic sequencing.
    EnableAutomaticSequencing(oneshot::Sender<bool>),
    /// Disable automatic sequencing.
    DisableAutomaticSequencing(oneshot::Sender<bool>),
    /// Enable gossiping of blocks to peers.
    #[cfg(feature = "test-utils")]
    SetGossip((bool, oneshot::Sender<()>)),
}
