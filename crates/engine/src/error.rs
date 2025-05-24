use alloy_json_rpc::RpcError;
use alloy_rpc_types_engine::PayloadError;
use alloy_transport::TransportErrorKind;

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
    /// The execution payload id is missing.
    #[error("missing payload id")]
    MissingExecutionPayloadId,
    /// The engine driver is syncing.
    #[error("Engine driver is syncing")]
    Syncing,
    /// Provider error.
    #[error(transparent)]
    ProviderRpc(#[from] RpcError<TransportErrorKind>),
}
