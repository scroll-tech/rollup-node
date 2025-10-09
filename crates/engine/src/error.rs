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
    #[error("head block number can not be below the safe block number")]
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
    pub const fn fcs_no_update_provided() -> Self {
        Self::FcsError(FcsError::NoUpdateProvided)
    }

    /// Creates a new [`EngineError`] for a [`FcsError::FinalizedBlockNumberNotIncreasing`].
    pub const fn fcs_finalized_block_number_not_increasing() -> Self {
        Self::FcsError(FcsError::FinalizedBlockNumberNotIncreasing)
    }
}
