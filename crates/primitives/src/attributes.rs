use crate::BatchInfo;
use std::sync::Arc;

use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;

/// The [`ScrollPayloadAttributes`] coupled with the batch information from which they originated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScrollPayloadAttributesWithBatchInfo {
    /// The payload attributes.
    pub payload_attributes: ScrollPayloadAttributes,
    /// The batch information from which the attributes originated.
    pub batch_info: Arc<BatchInfo>,
}

impl From<(ScrollPayloadAttributes, Arc<BatchInfo>)> for ScrollPayloadAttributesWithBatchInfo {
    fn from(value: (ScrollPayloadAttributes, Arc<BatchInfo>)) -> Self {
        Self { payload_attributes: value.0, batch_info: value.1 }
    }
}
