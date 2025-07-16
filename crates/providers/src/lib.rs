//! The crate exposes various Providers along with their implementations for usage across the rollup
//! node.

use alloy_provider::RootProvider;
use scroll_alloy_network::Scroll;

mod block;
pub use block::BlockDataProvider;

mod l1;
pub use l1::{
    blob::{
        AnvilBlobProvider, BlobProvider, BlobSource, ConsensusClientProvider, MockBeaconProvider,
    },
    message::{DatabaseL1MessageProvider, L1MessageProvider},
    system_contract::{SystemContractProvider, AUTHORIZED_SIGNER_STORAGE_SLOT},
    FullL1Provider, L1Provider, L1ProviderError,
};

/// An alias for a [`RootProvider`] using the [`Scroll`] network.
pub type ScrollRootProvider = RootProvider<Scroll>;
