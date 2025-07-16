//! The codec implementation for Scroll.

pub use block::{BlockContext, L2Block};
pub mod block;

pub mod decoding;

pub use error::{CodecError, DecodingError};
mod error;

use crate::decoding::{
    batch::Batch, v0::decode_v0, v1::decode_v1, v2::decode_v2, v4::decode_v4, v7::decode_v7,
};

use alloy_eips::eip4844::Blob;
use alloy_primitives::{ruint::UintTryTo, Bytes, U256};

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
    /// Decodes the input data and returns the decoded [`Batch`].
    pub fn decode<T: CommitDataSource>(input: &T) -> Result<Batch, CodecError> {
        let calldata = input.calldata();
        let version = get_codec_version(calldata)?;

        let payload = match version {
            0 => decode_v0(calldata)?,
            1 => {
                let blob = input.blob().ok_or(DecodingError::MissingBlob)?;
                decode_v1(calldata, blob.as_ref())?
            }
            2..4 => {
                let blob = input.blob().ok_or(DecodingError::MissingBlob)?;
                decode_v2(calldata, blob.as_ref())?
            }
            4..7 => {
                let blob = input.blob().ok_or(DecodingError::MissingBlob)?;
                decode_v4(calldata, blob.as_ref())?
            }
            7..=8 => {
                let blob = input.blob().ok_or(DecodingError::MissingBlob)?;
                decode_v7(blob.as_ref())?
            }
            v => return Err(DecodingError::UnsupportedCodecVersion(v).into()),
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

/// Returns the codec version from the calldata.
fn get_codec_version(calldata: &[u8]) -> Result<u8, DecodingError> {
    const CODEC_VERSION_OFFSET_START: usize = 4;
    const CODEC_VERSION_LEN: usize = 32;
    const CODEC_VERSION_OFFSET_END: usize = CODEC_VERSION_OFFSET_START + CODEC_VERSION_LEN;
    const HIGH_BYTES_MASK: U256 =
        U256::from_limbs([0xffffffffffffff00, u64::MAX, u64::MAX, u64::MAX]);

    let version = calldata
        .get(CODEC_VERSION_OFFSET_START..CODEC_VERSION_OFFSET_END)
        .ok_or(DecodingError::Eof)?;
    let version = U256::from_be_slice(version);

    if (version & HIGH_BYTES_MASK) != U256::ZERO {
        return Err(DecodingError::MalformedCodecVersion(version))
    }

    Ok(version.uint_try_to().expect("fits in single byte"))
}
