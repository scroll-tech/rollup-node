//! Test utilities for the engine crate.

use crate::EngineDriverError;

use alloy_rpc_types_engine::ExecutionPayload;
use rollup_node_providers::ExecutionPayloadProvider;

/// A default execution payload for testing that returns `Ok(None)` for all block IDs.
#[derive(Debug)]
pub struct NoopExecutionPayloadProvider;

#[async_trait::async_trait]
impl ExecutionPayloadProvider for NoopExecutionPayloadProvider {
    type Error = EngineDriverError;

    async fn execution_payload_by_block(
        &self,
        _block_id: alloy_eips::BlockId,
    ) -> Result<Option<ExecutionPayload>, Self::Error> {
        Ok(None)
    }
}
