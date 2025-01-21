/// Configuration for the ScrollWire Protocol.
#[derive(Debug, Clone)]
pub struct ScrollWireConfig {
    connect_unsupported_peer: bool,
}

impl ScrollWireConfig {
    /// Creates a new [`ScrollWireConfig`] with the provided configuration.
    pub fn new(connect_unsupported_peer: bool) -> Self {
        Self {
            connect_unsupported_peer,
        }
    }

    /// Returns a boolean indicating if the ScrollWire protocol should connect to peers that do not
    /// support the protocol.
    pub fn connect_unsupported_peer(&self) -> bool {
        self.connect_unsupported_peer
    }
}
