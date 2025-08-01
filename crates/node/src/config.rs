/// The configuration of the rollup node.
#[derive(Debug)]
pub struct Config {
    /// If it is a sequencer node.
    is_sequencer: bool,
    /// The block time in milliseconds.
    block_time: u64,
}

impl Config {
    /// Creates a new configuration.
    pub const fn new(is_sequencer: bool, block_time: u64) -> Self {
        Self { is_sequencer, block_time }
    }

    /// Returns a boolean representing if the node is a sequencer.
    pub const fn is_sequencer(&self) -> bool {
        self.is_sequencer
    }

    /// Returns the block time in milliseconds.
    pub const fn block_time(&self) -> u64 {
        self.block_time
    }
}
