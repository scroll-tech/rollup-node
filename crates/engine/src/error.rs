use alloy_rpc_types_engine::PayloadError;
use rollup_node_primitives::{ScrollPayloadAttributesWithBatchInfo, WithBlockNumber};
use scroll_alloy_provider::ScrollEngineApiError;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;

/// The error type for the engine API.
#[derive(Debug, thiserror::Error)]
pub enum EngineDriverError {
    /// The engine is unavailable.
    #[error("Engine is unavailable")]
    EngineUnavailable,
    /// The execution payload is invalid.
    #[error("Invalid execution payload: {0}")]
    InvalidExecutionPayload(#[from] PayloadError),
    /// The execution payload provider is unavailable.
    #[error("Execution payload provider is unavailable")]
    ExecutionPayloadProviderUnavailable,
    /// The forkchoice update failed.
    #[error("Forkchoice update failed: {0}")]
    ForkchoiceUpdateFailed(ScrollEngineApiError),
    /// The payload id field is missing in the forkchoice update response for an L1 consolidation
    /// job.
    #[error("Forkchoice update response missing payload id for L1 consolidation job")]
    L1ConsolidationMissingPayloadId(WithBlockNumber<ScrollPayloadAttributesWithBatchInfo>),
    /// The payload id field is missing in the forkchoice update response for a payload building
    /// job.
    #[error("Forkchoice update response missing payload id for payload building job")]
    PayloadBuildingMissingPayloadId(ScrollPayloadAttributes),
}
