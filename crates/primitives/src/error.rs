/// An error associated with a batch input.
#[derive(Debug)]
pub enum BatchError {
    /// The batch version is invalid.
    InvalidBatchVersion(u8),
}
