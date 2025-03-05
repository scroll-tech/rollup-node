use super::BatchInputVersion;
use alloy_primitives::B256;

/// The input data for a batch.
///
/// This is used as input for the derivation pipeline. All data remains in its raw serialized form.
/// The data is then deserialized, enriched and processed in the derivation pipeline.
#[derive(Clone, Debug, PartialEq)]
pub enum BatchInput {
    /// The input data for a batch.
    BatchInputV1(BatchInputV1),
    /// The input data for a batch including the L1 blob.
    BatchInputV2(BatchInputV2),
}

impl BatchInput {
    /// Returns the version of the batch input data.
    pub fn version(&self) -> BatchInputVersion {
        match self {
            BatchInput::BatchInputV1(data) => data.version,
            BatchInput::BatchInputV2(data) => data.batch_input_data.version,
        }
    }

    /// Returns the index of the batch.
    pub fn batch_index(&self) -> u64 {
        match self {
            BatchInput::BatchInputV1(data) => data.index,
            BatchInput::BatchInputV2(data) => data.batch_input_data.index,
        }
    }

    /// Returns the batch hash.
    pub fn batch_hash(&self) -> &B256 {
        match self {
            BatchInput::BatchInputV1(data) => &data.hash,
            BatchInput::BatchInputV2(data) => &data.batch_input_data.hash,
        }
    }

    /// Returns the L1 block number at which the batch was committed.
    pub fn block_number(&self) -> u64 {
        match self {
            BatchInput::BatchInputV1(data) => data.block_number,
            BatchInput::BatchInputV2(data) => data.batch_input_data.block_number,
        }
    }

    /// Returns the parent batch header.
    pub fn parent_batch_header(&self) -> &[u8] {
        match self {
            BatchInput::BatchInputV1(data) => &data.parent_batch_header,
            BatchInput::BatchInputV2(data) => &data.batch_input_data.parent_batch_header,
        }
    }

    /// Returns the chunks in the batch.
    pub fn chunks(&self) -> &[Vec<u8>] {
        match self {
            BatchInput::BatchInputV1(data) => &data.chunks,
            BatchInput::BatchInputV2(data) => &data.batch_input_data.chunks,
        }
    }

    /// Returns the skipped L1 message bitmap.
    pub fn skipped_l1_message_bitmap(&self) -> &[u8] {
        match self {
            BatchInput::BatchInputV1(data) => &data.skipped_l1_message_bitmap,
            BatchInput::BatchInputV2(data) => &data.batch_input_data.skipped_l1_message_bitmap,
        }
    }

    /// Returns the L1 blob hash associated with the batch.
    pub fn blob_hash(&self) -> Option<&B256> {
        match self {
            BatchInput::BatchInputV1(_) => None,
            BatchInput::BatchInputV2(data) => Some(&data.blob_hash),
        }
    }
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for BatchInput {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let version = u.arbitrary::<BatchInputVersion>()?;
        match version {
            BatchInputVersion::V1 => Ok(BatchInput::BatchInputV1(u.arbitrary()?)),
            BatchInputVersion::V2 => Ok(BatchInput::BatchInputV2(u.arbitrary()?)),
        }
    }
}

/// The input data for a batch.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchInputV1 {
    /// The version of the batch input data.
    pub version: BatchInputVersion,
    /// The index of the batch.
    pub index: u64,
    /// The batch hash.
    pub hash: B256,
    /// The L1 block number at which the batch was committed.
    pub block_number: u64,
    /// The parent batch header.
    pub parent_batch_header: Vec<u8>,
    /// The chunks in the batch.
    pub chunks: Vec<Vec<u8>>,
    /// The skipped L1 message bitmap.
    pub skipped_l1_message_bitmap: Vec<u8>,
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for BatchInputV1 {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let version = u.arbitrary::<BatchInputVersion>()?;
        let index = u.arbitrary::<u32>()? as u64;
        let hash = u.arbitrary::<B256>()?;
        let block_number = u.arbitrary::<u32>()? as u64;
        let parent_batch_header = u.arbitrary::<Vec<u8>>()?;
        let chunks = u.arbitrary::<Vec<Vec<u8>>>()?;
        let skipped_l1_message_bitmap = u.arbitrary::<Vec<u8>>()?;

        Ok(BatchInputV1 {
            version,
            index,
            hash,
            block_number,
            parent_batch_header,
            chunks,
            skipped_l1_message_bitmap,
        })
    }
}

/// The input data for a batch including the L1 blob hash.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchInputV2 {
    /// The base input data for the batch.
    pub batch_input_data: BatchInputV1,
    /// The L1 blob hash associated with the batch.
    pub blob_hash: B256,
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for BatchInputV2 {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(BatchInputV2 { batch_input_data: u.arbitrary()?, blob_hash: u.arbitrary()? })
    }
}
