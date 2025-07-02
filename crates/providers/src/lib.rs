//! The crate exposes various Providers along with their implementations for usage across the rollup
//! node.

use alloy_provider::RootProvider;
use scroll_alloy_network::Scroll;

mod beacon;

pub use beacon::{beacon_provider, BeaconProvider, OnlineBeaconClient};

mod block;
pub use block::BlockDataProvider;

mod l1;
pub use l1::{
    blob::L1BlobProvider,
    message::{DatabaseL1MessageProvider, L1MessageProvider},
    system_contract::{SystemContractProvider, AUTHORIZED_SIGNER_STORAGE_SLOT},
    L1Provider, L1ProviderError, OnlineL1Provider,
};

/// Test utils related to providers.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

/// An alias for a [`RootProvider`] using the [`Scroll`] network.
pub type ScrollRootProvider = RootProvider<Scroll>;
