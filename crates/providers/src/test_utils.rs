//! Test utils for providers.

use crate::{BlobProvider, L1MessageProvider, L1ProviderError};
use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use rollup_node_primitives::L1MessageEnvelope;
use scroll_db::{DatabaseError, DatabaseReadOperations, L1MessageKey};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// Implementation of the [`crate::L1Provider`] that returns blobs from a file.
#[derive(Clone, Default, Debug)]
pub struct MockL1Provider<DB: DatabaseReadOperations> {
    /// Database.
    pub db: DB,
    /// File blobs.
    pub blobs: HashMap<B256, PathBuf>,
}

#[async_trait::async_trait]
impl<DB: DatabaseReadOperations + Send + Sync> BlobProvider for MockL1Provider<DB> {
    async fn blob(
        &self,
        _block_timestamp: u64,
        hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        let blob = self.blobs.get(&hash).map(|path| {
            let blob = std::fs::read(path)
                .expect("failed to read blob file")
                .as_slice()
                .try_into()
                .expect("failed to convert bytes to blob");
            Arc::new(blob)
        });
        Ok(blob)
    }
}

#[async_trait::async_trait]
impl<DB: DatabaseReadOperations + Send + Sync> L1MessageProvider for MockL1Provider<DB> {
    type Error = DatabaseError;

    async fn get_n_messages(
        &self,
        start: L1MessageKey,
        n: u64,
    ) -> Result<Vec<L1MessageEnvelope>, Self::Error> {
        self.db.get_n_messages(start, n).await
    }
}
