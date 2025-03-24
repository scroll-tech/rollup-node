use crate::decoding::{v0::BatchHeaderV0, v1::BatchHeaderV1, v3::BatchHeaderV3, v7::BatchHeaderV7};

use crate::payload::PayloadData;
use alloy_primitives::B256;

/// A batch header.
#[derive(Debug, Clone, PartialEq, Eq, derive_more::From)]
pub enum BatchHeader {
    /// V0.
    V0(BatchHeaderV0),
    /// V1.
    V1(BatchHeaderV1),
    /// V3.
    V3(BatchHeaderV3),
    /// V7.
    V7(BatchHeaderV7),
}

impl BatchHeader {
    /// Returns the version of the batch.
    pub fn version(&self) -> u8 {
        match self {
            BatchHeader::V0(header) => header.version,
            BatchHeader::V1(header) => header.version,
            BatchHeader::V3(header) => header.version,
            BatchHeader::V7(header) => header.version,
        }
    }

    /// Returns the index of the batch.
    pub fn index(&self) -> u64 {
        match self {
            BatchHeader::V0(header) => header.batch_index,
            BatchHeader::V1(header) => header.batch_index,
            BatchHeader::V3(header) => header.batch_index,
            BatchHeader::V7(header) => header.batch_index,
        }
    }

    /// Returns the hash of the batch.
    pub fn hash(&self) -> &B256 {
        match self {
            BatchHeader::V0(header) => header.hash(),
            BatchHeader::V1(header) => header.hash(),
            BatchHeader::V3(header) => header.hash(),
            BatchHeader::V7(header) => header.hash(),
        }
    }

    /// Returns a new [`BatchHeader`], using the byte which contains the version in order to decide
    /// on the variant.
    pub fn try_from_buf(buf: &mut &[u8]) -> Option<Self> {
        let version = buf.first()?;
        match version {
            0 => Some(BatchHeader::V0(BatchHeaderV0::try_from_buf(buf)?)),
            1..3 => Some(BatchHeader::V1(BatchHeaderV1::try_from_buf(buf)?)),
            3..7 => Some(BatchHeader::V3(BatchHeaderV3::try_from_buf(buf)?)),
            // BatchHeaderV7 should not be built from a buffer.
            7 => None,
            _ => None,
        }
    }
}

/// The deserialized batch data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// The batch version.
    pub version: u8,
    /// The amount of blocks for each chunk of the batch. Only relevant for codec versions v0 ->
    /// v6.
    pub chunks_block_count: Option<Vec<usize>>,
    /// The data for the batch.
    pub data: PayloadData,
}

impl Batch {
    /// Returns a new instance of a batch.
    pub fn new(version: u8, chunks_block_count: Option<Vec<usize>>, data: PayloadData) -> Self {
        Self { version, chunks_block_count, data }
    }
}
