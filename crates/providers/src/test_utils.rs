//! Test utils for providers.

use crate::{BlobProvider, L1MessageProvider, L1ProviderError};
use std::{collections::HashMap, sync::Arc};

use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use rollup_node_primitives::L1MessageEnvelope;

/// Implementation of the [`crate::L1Provider`] that never returns blobs.
#[derive(Clone, Default, Debug)]
pub struct MockL1Provider<P: L1MessageProvider> {
    /// L1 message provider.
    pub l1_messages_provider: P,
    /// Mocked blobs.
    pub blobs: HashMap<B256, Blob>,
}

#[async_trait::async_trait]
impl<P: L1MessageProvider + Sync> BlobProvider for MockL1Provider<P> {
    async fn blob(
        &self,
        _block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        Ok(self.blobs.get(&hash).map(|b| Arc::new(*b)))
    }
}

#[async_trait::async_trait]
impl<P: L1MessageProvider + Send + Sync> L1MessageProvider for MockL1Provider<P> {
    type Error = P::Error;

    async fn take_n_messages_from_index(
        &self,
        start_index: u64,
        n: u64,
    ) -> Result<Vec<L1MessageEnvelope>, Self::Error> {
        self.l1_messages_provider.take_n_messages_from_index(start_index, n).await
    }

    async fn take_n_messages_from_hash(
        &self,
        queue_hash: B256,
        n: u64,
    ) -> Result<Vec<L1MessageEnvelope>, Self::Error> {
        self.l1_messages_provider.take_n_messages_from_hash(queue_hash, n).await
    }
}
