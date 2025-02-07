//! Test utilities for the engine crate.

use crate::EngineDriverError;

use super::ExecutionPayloadProvider;
use alloy_rpc_types_engine::ExecutionPayload;
use std::future::Future;

/// A default execution payload for testing that returns `Ok(None)` for all block IDs.
#[derive(Debug)]
pub struct NoopExecutionPayloadProvider;

impl ExecutionPayloadProvider for NoopExecutionPayloadProvider {
    fn execution_payload_by_block(
        &self,
        _block_id: alloy_eips::BlockId,
    ) -> impl Future<Output = Result<Option<ExecutionPayload>, EngineDriverError>> + Send {
        async move { Ok(None) }
    }
}
