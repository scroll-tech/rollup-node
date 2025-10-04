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

/// The error type for the fork choice state.
#[derive(Debug, thiserror::Error)]
pub enum FcsError {
    /// No update was provided for head, safe or finalized.
    #[error("No update provided for head, safe or finalized")]
    NoUpdateProvided,
    /// Finalized block number not increasing.
    #[error("Finalized block number not increasing")]
    FinalizedBlockNumberNotIncreasing,
    /// Head block number cannot be below safe block number.
    #[error("Safe block number can not be below the head block number")]
    HeadBelowSafe,
    /// Safe block number cannot be below finalized block number.
    #[error("Safe block number can not be below the finalized block number")]
    SafeBelowFinalized,
}

/// The error type for the Engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// An error occurred in the fork choice state.
    #[error("Fork choice state error: {0}")]
    FcsError(#[from] FcsError),
    /// An error occurred in the transport layer.
    #[error("Transport error: {0}")]
    TransportError(#[from] scroll_alloy_provider::ScrollEngineApiError),
}

impl EngineError {
    /// Creates a new [`EngineError`] for a [`FcsError::NoUpdateProvided`].
    pub fn fcs_no_update_provided() -> Self {
        Self::FcsError(FcsError::NoUpdateProvided)
    }

    /// Creates a new [`EngineError`] for a [`FcsError::FinalizedBlockNumberNotIncreasing`].
    pub fn fcs_finalized_block_number_not_increasing() -> Self {
        Self::FcsError(FcsError::FinalizedBlockNumberNotIncreasing)
    }
}
