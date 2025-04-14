/// The error type for the engine API.
#[derive(Debug, thiserror::Error)]
pub enum EngineDriverError {
    /// The engine is unavailable.
    #[error("Engine is unavailable")]
    EngineUnavailable,
    /// The execution payload is invalid.
    #[error("Invalid execution payload")]
    InvalidExecutionPayload,
    /// The engine failed to execute the fork choice update.
    #[error("Failed to execute fork choice update")]
    InvalidFcu,
    /// The execution payload provider is unavailable.
    #[error("Execution payload provider is unavailable")]
    ExecutionPayloadProviderUnavailable,
    /// The execution payload id is missing.
    #[error("missing payload id")]
    MissingExecutionPayloadId,
    /// The engine driver is syncing.
    #[error("Engine driver is syncing")]
    Syncing,
}
