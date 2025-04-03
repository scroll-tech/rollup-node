use alloy_eips::BlockId;
use alloy_rpc_types_engine::ExecutionPayload;

/// Implementers of the trait can provide the L2 execution payload for a block id.
#[async_trait::async_trait]
pub trait ExecutionPayloadProvider {
    /// The error returned by the provider.
    type Error;

    /// Returns the [`ExecutionPayload`] for the provided [`BlockId`], or [None].
    async fn execution_payload_by_block(
        &self,
        block_id: BlockId,
    ) -> Result<Option<ExecutionPayload>, Self::Error>;
}
