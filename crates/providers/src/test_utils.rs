use crate::{
    beacon::{APIResponse, ReducedConfigData, ReducedGenesisData},
    execution_payload::ExecutionPayloadProviderError,
    BeaconProvider, ExecutionPayloadProvider,
};

use alloy_rpc_types_beacon::sidecar::BlobData;
use alloy_rpc_types_engine::ExecutionPayload;

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

/// A default execution payload for testing that returns `Ok(None)` for all block IDs.
#[derive(Debug, Clone)]
pub struct NoopExecutionPayloadProvider;

#[async_trait::async_trait]
impl ExecutionPayloadProvider for NoopExecutionPayloadProvider {
    async fn execution_payload_by_block(
        &self,
        _block_id: alloy_eips::BlockId,
    ) -> Result<Option<ExecutionPayload>, ExecutionPayloadProviderError> {
        Ok(None)
    }
}
