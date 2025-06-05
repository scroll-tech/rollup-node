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
    use std::sync::Arc;

    use alloy_provider::ProviderBuilder;
    use alloy_rpc_client::RpcClient;
    use futures::StreamExt;
    use reth_scroll_chainspec::SCROLL_DEV;
    use rollup_node::test_utils::{
        default_sequencer_test_scroll_rollup_node_config, generate_tx, setup_engine,
    };
    use rollup_node_manager::RollupManagerEvent;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_should_get_execution_payload() -> eyre::Result<()> {
        // get a test node.
        let chain_spec = (*SCROLL_DEV).clone();
        let node_config = default_sequencer_test_scroll_rollup_node_config();
        let (mut nodes, _tasks, wallet) =
            setup_engine(node_config.clone(), 1, chain_spec.clone(), false).await.unwrap();

        let sequencer = nodes.pop().unwrap();
        let sequencer_rnm_handle = sequencer.inner.add_ons_handle.rollup_manager_handle.clone();
        let mut sequencer_events = sequencer_rnm_handle.get_event_listener().await.unwrap();

        // inject a transaction into the pool of the node.
        let tx = generate_tx(Arc::new(Mutex::new(wallet))).await;
        sequencer.rpc.inject_tx(tx).await.unwrap();

        // wait for the sequencer to build a block with the transaction.
        let block_number = loop {
            if let Some(RollupManagerEvent::BlockSequenced(block)) = sequencer_events.next().await {
                if block.body.transactions.len() > 0 {
                    break block.header.number;
                }
            }
        };

        // get a provider to the node.
        let url = sequencer.rpc_url();
        let client = RpcClient::new_http(url);
        let provider = ProviderBuilder::<_, _, Scroll>::default().connect_client(client);

        // fetch the execution payload for the first block.
        let payload = provider.execution_payload_for_block(block_number.into()).await?.unwrap();
        assert!(!payload.as_v1().transactions.is_empty());

        Ok(())
    }
}
