#[derive(Debug)]
pub enum EngineManagerError {
    /// The engine handle channel was closed.
    EngineHandleClosed,
}
