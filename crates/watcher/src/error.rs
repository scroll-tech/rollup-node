use alloy_json_rpc::RpcError;
use alloy_transport::TransportErrorKind;

/// A [`Result`] that uses [`L1WatcherError`] as the error type.
pub(crate) type L1WatcherResult<T> = Result<T, L1WatcherError>;

/// An error that occurred with the L1 watcher.
#[derive(Debug, thiserror::Error)]
pub enum L1WatcherError {
    #[error("execution provider error: {0:?}")]
    Provider(#[from] RpcError<TransportErrorKind>),
    #[error("missing block {0}")]
    MissingBlock(u64),
}
