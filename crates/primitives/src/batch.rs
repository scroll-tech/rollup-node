use std::sync::Arc;

use alloy_primitives::{Bytes, B256};

/// The batch information.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct BatchInfo {
    /// The index of the batch.
    pub index: u64,
    /// The hash of the batch.
    pub hash: B256,
}

impl BatchInfo {
    /// Returns a new instance of [`BatchInfo`].
    pub const fn new(index: u64, hash: B256) -> Self {
        Self { index, hash }
    }
}

/// The input data for a batch.
///
/// This is used as input for the derivation pipeline. All data remains in its raw serialized form.
/// The data is then deserialized, enriched and processed in the derivation pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchCommitData {
    /// The hash of the committed batch.
    pub hash: B256,
    /// The index of the batch.
    pub index: u64,
    /// The block number in which the batch was committed.
    pub block_number: u64,
    /// The block timestamp in which the batch was committed.
    pub block_timestamp: u64,
    /// The commit transaction calldata.
    pub calldata: Arc<Bytes>,
    /// The optional blob hash for the commit.
    pub blob_versioned_hash: Option<B256>,
}

impl From<BatchCommitData> for BatchInfo {
    fn from(value: BatchCommitData) -> Self {
        Self { index: value.index, hash: value.hash }
    }
}

#[cfg(feature = "arbitrary")]
mod arbitrary_impl {
    use super::*;

    impl arbitrary::Arbitrary<'_> for BatchCommitData {
        fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
            let batch_index = u.arbitrary::<u32>()? as u64;
            let batch_hash = u.arbitrary::<B256>()?;
            let block_number = u.arbitrary::<u32>()? as u64;
            let block_timestamp = u.arbitrary::<u32>()? as u64;
            let bytes = u.arbitrary::<Bytes>()?;
            let blob_hash = u.arbitrary::<bool>()?.then_some(u.arbitrary::<B256>()?);

            Ok(Self {
                hash: batch_hash,
                index: batch_index,
                block_number,
                block_timestamp,
                calldata: Arc::new(bytes),
                blob_versioned_hash: blob_hash,
            })
        }
    }
}
