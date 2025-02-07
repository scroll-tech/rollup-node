/// The error type for the engine API.
#[derive(Debug)]
pub enum EngineDriverError {
    /// The engine is unavailable.
    EngineUnavailable,
    /// The engine failed to validate the execution payload.
    InvalidExecutionPayload,
    /// The engine failed to execute the fork choice update.
    FcuFailed,
}
