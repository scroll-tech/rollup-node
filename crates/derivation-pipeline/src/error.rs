use scroll_codec::CodecError;

/// An error occurred during the derivation process.
#[derive(Debug, thiserror::Error)]
pub enum DerivationPipelineError {
    /// An error in the codec.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// Missing L1 messages cursor.
    #[error("missing l1 message queue cursor")]
    MissingL1MessageQueueCursor,
}
