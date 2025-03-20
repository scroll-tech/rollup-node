//! The codec implementation for Scroll.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc as std;

pub use block::{BlockContext, L2Block};
pub mod block;

pub mod decoding;

pub use error::CodecError;
mod error;

pub mod payload;

use crate::{
    decoding::{v0::decode_v0, v1::decode_v1, v2::decode_v2, v4::decode_v4, v7::decode_v7},
    error::DecodingError,
    payload::CommitPayload,
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
    /// V2 variant of the codec.
    V2,
    /// V3 variant of the codec.
    V3,
    /// V4 variant of the codec.
    V4,
    /// V5 variant of the codec.
    V5,
    /// V6 variant of the codec.
    V6,
    /// V7 variant of the codec.
    V7,
}

impl Codec {
    /// Decodes the input data and returns [`Comig`]
    pub fn decode<T: CommitDataSource>(input: T) -> Result<CommitPayload, CodecError> {
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
