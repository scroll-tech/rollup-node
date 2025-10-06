//! Test utils for providers.

use crate::{BlobProvider, L1MessageProvider, L1ProviderError};
use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use rollup_node_primitives::L1MessageEnvelope;
use scroll_db::L1MessageKey;
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// Implementation of the [`crate::L1Provider`] that returns blobs from a file.
#[derive(Clone, Default, Debug)]
pub struct MockL1Provider<P: L1MessageProvider> {
    /// L1 message provider.
    pub l1_messages_provider: P,
    /// File blobs.
    pub blobs: HashMap<B256, PathBuf>,
}

#[async_trait::async_trait]
impl<P: L1MessageProvider + Sync> BlobProvider for MockL1Provider<P> {
    async fn blob(
        &self,
        _block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        let blob = self.blobs.get(&hash).map(|path| {
            let arr = std::fs::read(path)
                .expect("failed to read blob file")
                .as_slice()
                .try_into()
                .expect("failed to convert bytes to blob");
            Arc::new(arr)
        });
        Ok(blob)
    }
}

#[async_trait::async_trait]
impl<P: L1MessageProvider + Send + Sync> L1MessageProvider for MockL1Provider<P> {
    type Error = P::Error;

    async fn get_n_messages(
        &self,
        start: L1MessageKey,
        n: u64,
    ) -> Result<Vec<L1MessageEnvelope>, Self::Error> {
        self.l1_messages_provider.get_n_messages(start, n).await
    }
}
