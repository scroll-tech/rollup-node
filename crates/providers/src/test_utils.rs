use crate::{
    beacon::{APIResponse, ReducedConfigData, ReducedGenesisData},
    BeaconProvider, L1BlobProvider, L1MessageProvider, L1ProviderError,
};
use std::sync::Arc;

use alloy_eips::eip4844::Blob;
use alloy_primitives::B256;
use alloy_rpc_types_beacon::sidecar::BlobData;
use rollup_node_primitives::L1MessageEnvelope;

/// Mocks all calls to the beacon chain.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct MockBeaconProvider;

#[async_trait::async_trait]
impl BeaconProvider for MockBeaconProvider {
    type Error = reqwest::Error;

    async fn config_spec(&self) -> Result<APIResponse<ReducedConfigData>, Self::Error> {
        Ok(APIResponse { data: ReducedConfigData::default() })
    }

    async fn beacon_genesis(&self) -> Result<APIResponse<ReducedGenesisData>, Self::Error> {
        Ok(APIResponse { data: ReducedGenesisData::default() })
    }

    async fn blobs(&self, _slot: u64) -> Result<Vec<BlobData>, Self::Error> {
        Ok(vec![])
    }
}

/// Implementation of the [`crate::L1Provider`] that never returns blobs.
#[derive(Clone, Debug)]
pub struct NoBlobProvider<P: L1MessageProvider> {
    /// L1 message provider.
    pub l1_messages_provider: P,
}

#[async_trait::async_trait]
impl<P: L1MessageProvider + Sync> L1BlobProvider for NoBlobProvider<P> {
    async fn blob(
        &self,
        _block_timestamp: u64,
        _hash: B256,
    ) -> Result<Option<Arc<Blob>>, L1ProviderError> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl<P: L1MessageProvider + Send + Sync> L1MessageProvider for NoBlobProvider<P> {
    type Error = P::Error;

    async fn get_l1_message_with_block_number(
        &self,
    ) -> Result<Option<L1MessageEnvelope>, Self::Error> {
        self.l1_messages_provider.get_l1_message_with_block_number().await
    }
    fn set_queue_index_cursor(&self, index: u64) {
        self.l1_messages_provider.set_queue_index_cursor(index);
    }
    async fn set_hash_cursor(&self, hash: B256) {
        self.l1_messages_provider.set_hash_cursor(hash).await
    }
    fn increment_cursor(&self) {
        self.l1_messages_provider.increment_cursor()
    }
}
