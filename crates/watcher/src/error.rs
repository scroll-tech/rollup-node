use alloy_json_rpc::RpcError;
use alloy_primitives::B256;
use alloy_transport::TransportErrorKind;

/// A [`Result`] that uses [`L1WatcherError`] as the error type.
pub(crate) type L1WatcherResult<T> = Result<T, L1WatcherError>;

/// An error that occurred with the L1 watcher.
#[derive(Debug, thiserror::Error)]
pub enum L1WatcherError {
    /// A Provider error at the RPC level.
    #[error("execution provider rpc error: {0:?}")]
    ProviderRpc(#[from] RpcError<TransportErrorKind>),
    /// An Ethereum request error.
    #[error("failed Ethereum JSON RPC request: {0:?}")]
    EthRequest(#[from] EthRequestError),
    /// An error related to logs in the L1 watcher.
    #[error(transparent)]
    Logs(#[from] FilterLogError),
}

/// An error occurred during a request to the Ethereum JSON RPC provider.
#[derive(Debug, thiserror::Error)]
pub enum EthRequestError {
    /// The requested block does not exist.
    #[error("unknown block {0}")]
    MissingBlock(u64),
    /// The requested transaction hash does not exist.
    #[error("unknown transaction {0}")]
    MissingTransactionHash(B256),
}

/// An error that occurred when filtering logs.
#[derive(Debug, thiserror::Error)]
pub enum FilterLogError {
    /// The log is missing a block number.
    #[error("missing block number for log")]
    MissingBlockNumber,
    /// The log is missing a block timestamp.
    #[error("missing block timestamp for log")]
    MissingBlockTimestamp,
    /// The log is missing a transaction hash.
    #[error("unknown transaction hash for log")]
    MissingTransactionHash,
}
