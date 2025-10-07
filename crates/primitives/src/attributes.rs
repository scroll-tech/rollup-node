use crate::BatchInfo;
use alloy_primitives::BlockNumber;

use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;

/// The [`ScrollPayloadAttributes`] coupled with the batch information from which they originated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScrollPayloadAttributesWithBatchInfo {
    /// The payload attributes.
    pub payload_attributes: ScrollPayloadAttributes,
    /// The batch information from which the attributes originated.
    pub batch_info: BatchInfo,
    /// The L2 block number that will be constructed with this payload.
    pub l2_block_number: BlockNumber,
}

impl ScrollPayloadAttributesWithBatchInfo {
    /// Returns a new instance of a [`ScrollPayloadAttributesWithBatchInfo`].
    pub const fn new(
        attributes: ScrollPayloadAttributes,
        batch_info: BatchInfo,
        l2_block_number: BlockNumber,
    ) -> Self {
        Self { payload_attributes: attributes, batch_info, l2_block_number }
    }
}
