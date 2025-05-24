use std::fmt::Debug;

use alloy_eips::BlockId;
use alloy_provider::Provider;
use alloy_rpc_types_engine::ExecutionPayload;
use alloy_transport::{RpcError, TransportErrorKind};
use scroll_alloy_network::Scroll;

#[derive(Debug, thiserror::Error)]
pub enum ExecutionPayloadProviderError {
    /// An error occurred at the transport layer.
    #[error("transport error: {0}")]
    Rpc(#[from] RpcError<TransportErrorKind>),
}

/// Implementers of the trait can provide the L2 execution payload for a block id.
#[async_trait::async_trait]
pub trait ExecutionPayloadProvider: Sync + Send {
    /// Returns the [`ExecutionPayload`] for the provided [`BlockId`], or [None].
    async fn execution_payload_for_block(
        &self,
        block_id: BlockId,
    ) -> Result<Option<ExecutionPayload>, ExecutionPayloadProviderError>;
}

#[async_trait::async_trait]
impl<P: Provider<Scroll>> ExecutionPayloadProvider for P {
    async fn execution_payload_for_block(
        &self,
        block_id: BlockId,
    ) -> Result<Option<ExecutionPayload>, ExecutionPayloadProviderError> {
        tracing::trace!(target: "scroll::providers", ?block_id, "fetching execution payload");

        let block = self.get_block(block_id).full().await?;
        Ok(block.map(|b| {
            ExecutionPayload::from_block_slow(
                &b.into_consensus().map_transactions(|tx| tx.inner.into_inner()),
            )
            .0
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use alloy_provider::ProviderBuilder;
    use alloy_rpc_client::RpcClient;
    use reth_e2e_test_utils::setup_engine;
    use reth_payload_primitives::PayloadBuilderAttributes;
    use reth_scroll_node::{ScrollNode, ScrollPayloadBuilderAttributes};
    use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
    use scroll_network::SCROLL_MAINNET;

    #[tokio::test]
    async fn test_should_get_execution_payload() -> eyre::Result<()> {
        let chain_spec = (*SCROLL_MAINNET).clone();

        // Get a test node.
        let (mut node, _tasks, _wallet) =
            setup_engine::<ScrollNode>(1, chain_spec, false, scroll_payload_attributes).await?;
        let node = node.pop().unwrap();

        // Get a provider to the node.
        let url = node.rpc_url();
        let client = RpcClient::new_http(url);
        let provider = ProviderBuilder::<_, _, Scroll>::default().connect_client(client);

        // Fetch the execution payload for the first block.
        let payload = provider.execution_payload_for_block(0.into()).await?;
        assert!(payload.is_some());

        Ok(())
    }

    /// Helper function to create a new eth payload attributes
    fn scroll_payload_attributes(_timestamp: u64) -> ScrollPayloadBuilderAttributes {
        let attributes = ScrollPayloadAttributes::default();
        ScrollPayloadBuilderAttributes::try_new(B256::ZERO, attributes, 0).unwrap()
    }
}
