use crate::{
    beacon::{APIResponse, ReducedConfigData, ReducedGenesisData},
    BeaconProvider,
};

use alloy_rpc_types_beacon::sidecar::BlobData;

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
