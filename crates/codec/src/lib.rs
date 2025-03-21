//! The codec implementation for Scroll.

pub use block::{BlockContext, L2Block};
pub mod block;

pub mod decoding;

pub use error::CodecError;
mod error;

pub use payload::CommitPayload;
pub mod payload;

use crate::{
    decoding::{v0::decode_v0, v1::decode_v1, v2::decode_v2, v4::decode_v4, v7::decode_v7},
    error::DecodingError,
};

use alloy_eips::eip4844::Blob;
use alloy_primitives::Bytes;

/// The Codec.
#[derive(Debug)]
pub enum Codec {
    /// V0 variant of the codec.
    /// <https://github.com/scroll-tech/scroll-contracts/blob/81f0db72ca5335e0dddfaa99cb415e3d1cee895f/src/libraries/codec/ChunkCodecV0.sol>
    V0,
    /// V1 variant of the codec.
    /// <https://github.com/scroll-tech/scroll-contracts/blob/81f0db72ca5335e0dddfaa99cb415e3d1cee895f/src/libraries/codec/ChunkCodecV1.sol>
    V1,
    /// V2 variant of the codec. Similar to V1 with additional zstd encoding of the data.
    V2,
    /// V3 variant of the codec. Similar to V2 in regard to decoding.
    V3,
    /// V4 variant of the codec. Data can either be plain as in V1 or zstd encoded as in V2.
    V4,
    /// V5 variant of the codec. Similar to V4 in regard to decoding.
    V5,
    /// V6 variant of the codec. Similar to V4 in regard to decoding.
    V6,
    /// V7 variant of the codec. All data of interest is moved from the calldata to the blob.
    /// <https://github.com/scroll-tech/da-codec/blob/main/encoding/codecv7_types.go#L33>
    V7,
}

impl Codec {
    /// Decodes the input data and returns the decoded [`CommitPayload`].
    pub fn decode<T: CommitDataSource>(input: &T) -> Result<CommitPayload, CodecError> {
        let calldata = input.calldata();
        let version = calldata.first().ok_or(DecodingError::MissingCodecVersion)?;

        let payload = match version {
            0 => decode_v0(calldata)?.into(),
            1 => {
                let blob = input.blob().ok_or(DecodingError::MissingBlob)?;
                decode_v1(calldata, blob.as_ref())?.into()
            }
            2..4 => {
                let blob = input.blob().ok_or(DecodingError::MissingBlob)?;
                decode_v2(calldata, blob.as_ref())?.into()
            }
            4..7 => {
                let blob = input.blob().ok_or(DecodingError::MissingBlob)?;
                decode_v4(calldata, blob.as_ref())?.into()
            }
            7 => {
                let blob = input.blob().ok_or(DecodingError::MissingBlob)?;
                decode_v7(blob.as_ref())?.into()
            }
            v => return Err(DecodingError::UnsupportedCodecVersion(*v).into()),
        };

        Ok(payload)
    }
}

/// Values that implement the trait can provide data from a transaction's calldata or blob.
pub trait CommitDataSource {
    /// Returns the calldata from the commit transaction.
    fn calldata(&self) -> &Bytes;
    /// Returns the blob for decoding.
    fn blob(&self) -> Option<&Blob>;
}
