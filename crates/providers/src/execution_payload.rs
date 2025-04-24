use alloy_eips::BlockId;
use alloy_provider::Provider;
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

/// The provider uses an [`Provider`] internally to implement the [`ExecutionPayloadProvider`]
/// trait.
#[derive(Default, Clone, Debug)]
pub struct AlloyExecutionPayloadProvider<P> {
    /// An alloy provider.
    provider: P,
}

impl<P: Provider> AlloyExecutionPayloadProvider<P> {
    /// Returns a new instance of a [`AlloyExecutionPayloadProvider`].
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl<P: Provider> ExecutionPayloadProvider for AlloyExecutionPayloadProvider<P> {
    type Error = Box<dyn std::error::Error>;

    async fn execution_payload_by_block(
        &self,
        block_id: BlockId,
    ) -> Result<Option<ExecutionPayload>, Self::Error> {
        let block = self.provider.get_block(block_id).await?;
        Ok(block.map(|b| {
            ExecutionPayload::from_block_slow(
                &b.into_consensus().map_transactions(|tx| tx.inner.into_inner()),
            )
            .0
        }))
    }
}
