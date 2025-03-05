use alloy_primitives::B256;
use derive_more;

/// The input data for a batch.
///
/// This is used as input for the derivation pipeline. All data remains in its raw serialized form.
/// The data is then deserialized, enriched and processed in the derivation pipeline.
#[derive(Debug, derive_more::From)]
pub enum BatchInput {
    /// The input data for a batch.
    BatchInputDataV1(BatchInputV1),
    /// The input data for a batch including the L1 blob.
    BatchInputDataV2(BatchInputV2),
}

/// The input data for a batch.
#[derive(Debug)]
pub struct BatchInputV1 {
    /// The version of the batch input data.
    pub version: u8,
    /// The index of the batch.
    pub batch_index: u64,
    /// The batch hash.
    pub batch_hash: B256,
    /// The L1 block number at which the batch was committed.
    pub block_number: u64,
    /// The parent batch header.
    pub parent_batch_header: Vec<u8>,
    /// The chunks in the batch.
    pub chunks: Vec<Vec<u8>>,
    /// The skipped L1 message bitmap.
    pub skipped_l1_message_bitmap: Vec<u8>,
}

/// The input data for a batch including the L1 blob hash.
#[derive(Debug)]
pub struct BatchInputV2 {
    /// The base input data for the batch.
    pub batch_input_data: BatchInputV1,
    /// The L1 blob hash associated with the batch.
    pub blob_hash: B256,
}

/// A builder for the batch input. Determines the batch version based on the passed input.
#[derive(Debug, Default)]
pub struct BatchInputBuilder {
    /// The version of the batch input data.
    version: Option<u8>,
    /// The index of the batch.
    batch_index: Option<u64>,
    /// The batch hash.
    batch_hash: Option<B256>,
    /// The L1 block number at which the batch was committed.
    block_number: Option<u64>,
    /// The parent batch header.
    parent_batch_header: Option<Vec<u8>>,
    /// The chunks in the batch.
    chunks: Option<Vec<Vec<u8>>>,
    /// The skipped L1 message bitmap.
    skipped_l1_message_bitmap: Option<Vec<u8>>,
    /// The L1 blob hashes for the batch
    blob_hashes: Option<Vec<B256>>,
}

impl BatchInputBuilder {
    /// Adds a version to the builder.
    pub fn with_version(mut self, version: u8) -> Self {
        self.version = Some(version);
        self
    }

    /// Adds a batch index to the builder.
    pub fn with_batch_index(mut self, batch_index: u64) -> Self {
        self.batch_index = Some(batch_index);
        self
    }

    /// Adds a batch hash to the builder.
    pub fn with_batch_hash(mut self, batch_hash: B256) -> Self {
        self.batch_hash = Some(batch_hash);
        self
    }

    /// Adds a block number for the batch to the builder.
    pub fn with_block_number(mut self, block_number: u64) -> Self {
        self.block_number = Some(block_number);
        self
    }

    /// Adds a parent batch header to the builder.
    pub fn with_parent_batch_header(mut self, parent_batch_header: Vec<u8>) -> Self {
        self.parent_batch_header = Some(parent_batch_header);
        self
    }

    /// Adds chunks to the builder.
    pub fn with_chunks(mut self, chunks: Vec<Vec<u8>>) -> Self {
        self.chunks = Some(chunks);
        self
    }

    /// Adds skipped l1 message bitmap to the builder.
    pub fn with_skipped_l1_message_bitmap(mut self, skipped_l1_message_bitmap: Vec<u8>) -> Self {
        self.skipped_l1_message_bitmap = Some(skipped_l1_message_bitmap);
        self
    }

    /// Adds a blob hash to the builder.
    pub fn with_blob_hashes(mut self, blob_hashes: Option<Vec<B256>>) -> Self {
        self.blob_hashes = blob_hashes;
        self
    }

    /// Build the [`BatchInput`], returning [`None`] if fields haven't been correctly set.
    pub fn try_build(self) -> Option<BatchInput> {
        // handle fields required for all batch inputs.
        let version = self.version?;
        let batch_index = self.batch_index?;
        let batch_hash = self.batch_hash?;
        let block_number = self.block_number?;
        let parent_batch_header = self.parent_batch_header?;

        match (self.chunks, self.skipped_l1_message_bitmap, self.blob_hashes) {
            (Some(chunks), Some(skipped_l1_message_bitmap), None) => Some(
                BatchInputV1 {
                    version,
                    batch_index,
                    batch_hash,
                    block_number,
                    parent_batch_header,
                    chunks,
                    skipped_l1_message_bitmap,
                }
                .into(),
            ),
            (Some(chunks), Some(skipped_l1_message_bitmap), Some(blob)) => {
                let batch_input_data = BatchInputV1 {
                    version,
                    batch_index,
                    batch_hash,
                    block_number,
                    parent_batch_header,
                    chunks,
                    skipped_l1_message_bitmap,
                };
                let blob_hash = blob.first().cloned()?;
                Some(BatchInputV2 { batch_input_data, blob_hash }.into())
            }
            (None, None, Some(blobs)) => {
                // TODO(greg): for now None but this will be used in Euclid.
                None
            }
            _ => None,
        }
    }
}
