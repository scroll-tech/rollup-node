/// The error type for the engine API.
#[derive(Debug, thiserror::Error)]
pub enum EngineDriverError {
    /// The engine is unavailable.
    #[error("engine is unavailable")]
    EngineUnavailable,
    /// The execution payload is invalid.
    #[error("invalid execution payload")]
    InvalidExecutionPayload,
    /// The engine failed to execute the fork choice update.
    #[error("invalid forkchoice update")]
    InvalidFcu,
    /// The execution payload provider is unavailable.
    #[error("execution payload provider unavailable")]
    ExecutionPayloadProviderUnavailable,
    /// The engine driver is syncing.
    #[error("engine driver is syncing")]
    Syncing,
}
