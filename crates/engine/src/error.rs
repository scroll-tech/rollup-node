/// The error type for the engine API.
#[derive(Debug)]
pub enum EngineDriverError {
    /// The engine is unavailable.
    EngineUnavailable,
    /// The execution payload is invalid.
    InvalidExecutionPayload,
    /// The engine failed to execute the fork choice update.
    InvalidFcu,
    /// The execution payload provider is unavailable.
    ExecutionPayloadProviderUnavailable,
    /// The engine driver is syncing.
    Syncing,
}
