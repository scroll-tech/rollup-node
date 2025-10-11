mod event;
pub use event::{NewBlockWithPeer, ScrollNetworkManagerEvent};

mod handle;
pub use handle::{NetworkHandleMessage, ScrollNetworkHandle};

mod import;
pub use import::{
    BlockImportError, BlockImportOutcome, BlockImportResult, BlockValidation, BlockValidationError,
    ConsensusError,
};

mod manager;
pub use manager::ScrollNetworkManager;

pub use reth_network::{EthNetworkPrimitives, NetworkConfigBuilder};
pub use reth_scroll_chainspec::SCROLL_MAINNET;
use reth_tokio_util::EventStream;
pub use scroll_wire::ScrollWireConfig;

/// The main network struct that encapsulates the network handle and event stream.
#[derive(Debug)]
pub struct ScrollNetwork<N> {
    /// The network handle to interact with the network manager.
    handle: ScrollNetworkHandle<N>,
    /// Event stream for network manager events.
    events: EventStream<ScrollNetworkManagerEvent>,
}

impl<N> ScrollNetwork<N> {
    /// Creates a new instance of `ScrollNetwork`.
    pub fn handle(&self) -> &ScrollNetworkHandle<N> {
        &self.handle
    }

    /// Returns a mutable reference to the event stream.
    pub fn events(&mut self) -> &mut EventStream<ScrollNetworkManagerEvent> {
        &mut self.events
    }
}
