//! Test utilities for the engine crate.

use alloy_primitives::{BlockHash, U64};
use alloy_rpc_types_engine::{
    ClientVersionV1, ExecutionPayloadBodiesV1, ExecutionPayloadV1, ForkchoiceState,
    ForkchoiceUpdated, PayloadId, PayloadStatus,
};
use scroll_alloy_provider::{ScrollEngineApi, ScrollEngineApiResult};
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;

/// A [`ScrollEngineApi`] implementation that panics when any method is called.
#[derive(Debug)]
pub struct PanicEngineClient;

#[async_trait::async_trait]
impl ScrollEngineApi for PanicEngineClient {
    async fn new_payload_v1(
        &self,
        _payload: ExecutionPayloadV1,
    ) -> ScrollEngineApiResult<PayloadStatus> {
        panic!("PanicEngineClient does not support new_payload_v1")
    }

    async fn fork_choice_updated_v1(
        &self,
        _fork_choice_state: ForkchoiceState,
        _payload_attributes: Option<ScrollPayloadAttributes>,
    ) -> ScrollEngineApiResult<ForkchoiceUpdated> {
        panic!("PanicEngineClient does not support fork_choice_updated_v1")
    }

    async fn get_payload_v1(
        &self,
        _payload_id: PayloadId,
    ) -> ScrollEngineApiResult<ExecutionPayloadV1> {
        panic!("PanicEngineClient does not support get_payload_v1")
    }

    async fn get_payload_bodies_by_hash_v1(
        &self,
        _block_hashes: Vec<BlockHash>,
    ) -> ScrollEngineApiResult<ExecutionPayloadBodiesV1> {
        panic!("PanicEngineClient does not support get_payload_bodies_by_hash_v1")
    }

    async fn get_payload_bodies_by_range_v1(
        &self,
        _start: U64,
        _count: U64,
    ) -> ScrollEngineApiResult<ExecutionPayloadBodiesV1> {
        panic!("PanicEngineClient does not support get_payload_bodies_by_range_v1")
    }

    async fn get_client_version_v1(
        &self,
        _client_version: ClientVersionV1,
    ) -> ScrollEngineApiResult<Vec<ClientVersionV1>> {
        panic!("PanicEngineClient does not support get_client_version_v1")
    }

    async fn exchange_capabilities(
        &self,
        _capabilities: Vec<String>,
    ) -> ScrollEngineApiResult<Vec<String>> {
        panic!("PanicEngineClient does not support exchange_capabilities")
    }
}
