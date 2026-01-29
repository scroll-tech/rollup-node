//! An implementation of the scroll-wire protocol.

mod config;
pub use config::ScrollWireConfig;

mod connection;
mod manager;
pub use manager::{PeerState, ScrollWireManager, LRU_CACHE_SIZE};

mod protocol;
pub use protocol::{NewBlock, ScrollMessage, ScrollWireEvent, ScrollWireProtocolHandler};
