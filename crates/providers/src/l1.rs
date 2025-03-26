use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use kona_providers_alloy::{BeaconClient, OnlineBlobProvider};
use scroll_alloy_consensus::TxL1Message;

/// An instance of the trait can provide L1 messages using a cursor approach. Set the cursor for the
/// provider using the queue index or hash and then call [`L1MessageProvider::next_l1_message`] to
/// iterate the queue.
pub trait L1MessageProvider {
    /// Returns the L1 message at the current cursor and advances the cursor.
    fn next_l1_message(&self) -> TxL1Message;
    /// Set the index cursor for the provider.
    fn set_index_cursor(&mut self, index: u64);
    /// Set the hash cursor for the provider.
    fn set_hash_cursor(&mut self, hash: B256);
}

/// An instance of the trait can be used to provide L1 data.
pub trait L1Provider: L1MessageProvider {
    /// Returns corresponding blob data for the provided hash.
    fn blob(&self, hash: B256) -> Option<Blob>;
}

/// An online implementation of the [`L1Provider`] trait.
#[derive(Debug)]
pub struct OnlineL1Provider<C: BeaconClient> {
    /// The blob provider.
    blob_provider: OnlineBlobProvider<C>,
}

impl<C: BeaconClient> OnlineL1Provider<C> {
    /// Returns a new [`OnlineL1Provider`] from the provided [`BeaconClient`].
    pub async fn new(client: C) -> Self {
        let blob_provider = OnlineBlobProvider::init(client).await;
        Self { blob_provider }
    }
}
