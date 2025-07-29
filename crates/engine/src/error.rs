use alloy_rpc_types_engine::PayloadError;
use rollup_node_primitives::ScrollPayloadAttributesWithBatchInfo;
use scroll_alloy_provider::ScrollEngineApiError;

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
    /// The payload id field is missing in the forkchoice update response.
    #[error("Forkchoice update response missing payload id")]
    MissingPayloadId(ScrollPayloadAttributesWithBatchInfo),
}
