use alloy_primitives::U256;
use scroll_l1::abi::calls::InvalidCommitBatchCall;

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
    #[error("unsupported codec version {0}")]
    UnsupportedCodecVersion(u8),
    #[error("malformed codec version: {0}")]
    MalformedCodecVersion(U256),
    #[error("missing blob from data source")]
    MissingBlob,
    #[error("missing chunk data")]
    MissingChunkData,
    #[error("missing parent header data")]
    MissingParentHeader,
    #[error("invalid calldata format")]
    InvalidCalldataFormat,
    #[error("invalid parent header format")]
    InvalidParentHeaderFormat,
    #[error(transparent)]
    InvalidCommitBatchCall(#[from] InvalidCommitBatchCall),
    #[error("end of file")]
    Eof,
    #[error("zstd decompression error occurred: {0}")]
    ZstdDecompression(Box<dyn std::error::Error + Send + Sync + 'static>),
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

impl From<crate::decoding::v2::zstd::ZstdError> for DecodingError {
    fn from(e: crate::decoding::v2::zstd::ZstdError) -> Self {
        DecodingError::ZstdDecompression(e.into_inner())
    }
}
