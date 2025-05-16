//! The crate exposes various Providers along with their implementations for usage across the rollup
//! node.

pub use beacon::{beacon_provider, BeaconProvider, OnlineBeaconClient};
mod beacon;

pub use block::BlockDataProvider;
mod block;

pub use execution_payload::{AlloyExecutionPayloadProvider, ExecutionPayloadProvider};
mod execution_payload;

pub use l1::{
    blob::L1BlobProvider,
    message::{DatabaseL1MessageProvider, L1MessageProvider},
    system_contract::{SystemContractProvider, AUTHORIZED_SIGNER_STORAGE_SLOT},
    L1Provider, L1ProviderError, OnlineL1Provider,
};
mod l1;

/// Test utils related to providers.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
