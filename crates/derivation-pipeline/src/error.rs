use scroll_codec::CodecError;

/// An error that occurred in the derivation pipeline
#[derive(Debug, thiserror::Error)]
pub enum DerivationPipelineError {
    /// An error occurred at the codec level.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// Invalid calldata format in the pipeline.
    #[error("invalid calldata format")]
    InvalidCalldataFormat,
    #[error("failed to decode batch header")]
    InvalidBatchHeader,
}
