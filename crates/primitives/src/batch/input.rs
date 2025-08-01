use alloy_primitives::B256;

/// The input data for a batch.
///
/// This is used as input for the derivation pipeline. All data remains in its raw serialized form.
/// The data is then deserialized, enriched and processed in the derivation pipeline.
#[derive(Debug)]
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
