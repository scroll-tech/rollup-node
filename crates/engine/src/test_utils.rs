use crate::ExecutionPayloadProvider;
use alloy_eips::BlockId;
use alloy_rpc_types_engine::ExecutionPayload;

impl ExecutionPayloadProvider for () {
    async fn execution_payload_by_block(
        &self,
        _block_id: BlockId,
    ) -> eyre::Result<Option<ExecutionPayload>> {
        Ok(None)
    }
}
