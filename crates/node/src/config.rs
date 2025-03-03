/// The configuration of the rollup node.
#[derive(Debug)]
pub struct Config {
    /// If it is a sequencer node.
    is_sequencer: bool,
    /// The block time in milliseconds.
    block_time: u64,
}
