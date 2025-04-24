/// An enum representing the errors that can occur in the signer.
#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    /// An error occurred while signing.
    #[error("Failed to sign: {0}")]
    SigningError(#[from] alloy_signer::Error),
    /// The Signer request channel was closed.
    #[error("Request channel closed")]
    RequestChannelClosed,
    /// The Signer event channel was closed.
    #[error("Event channel closed")]
    EventChannelClosed,
}
