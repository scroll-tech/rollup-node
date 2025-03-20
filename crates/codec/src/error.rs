/// An error occurring during the codec process.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// An error occurring at the decoding state.
    #[error(transparent)]
    Decoding(#[from] DecodingError),
}

/// An error occurring during the decoding.
#[derive(Debug, thiserror::Error)]
pub enum DecodingError {
    #[error("missing codec version in input")]
    MissingCodecVersion,
    #[error("unsupported codec version {0}")]
    UnsupportedCodecVersion(u8),
    #[error("missing blob from data source")]
    MissingBlob,
    #[error("invalid calldata format")]
    InvalidCalldataFormat,
    #[error("end of file")]
    Eof,
    #[error("decoding error occurred: {0}")]
    Other(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl DecodingError {
    /// Returns a [`DecodingError::Other`] error from the provided string.
    pub fn other(msg: String) -> Self {
        Self::Other(msg.into())
    }
}

impl From<String> for DecodingError {
    fn from(value: String) -> Self {
        DecodingError::Other(value.into())
    }
}
