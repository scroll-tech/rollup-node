use std::fmt::Debug;

use alloy_eips::BlockId;
use alloy_provider::Provider;
use alloy_rpc_types_engine::ExecutionPayload;
use alloy_transport::{RpcError, TransportErrorKind};

#[derive(Debug, thiserror::Error)]
pub enum ExecutionPayloadProviderError {
    /// An error occurred at the transport layer.
    #[error("transport error: {0}")]
    Rpc(#[from] RpcError<TransportErrorKind>),
}

/// Implementers of the trait can provide the L2 execution payload for a block id.
#[async_trait::async_trait]
#[auto_impl::auto_impl(Arc)]
pub trait ExecutionPayloadProvider: Sync + Send {
    /// Returns the [`ExecutionPayload`] for the provided [`BlockId`], or [None].
    async fn execution_payload_by_block(
        &self,
        block_id: BlockId,
    ) -> Result<Option<ExecutionPayload>, ExecutionPayloadProviderError>;
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
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl<P: Provider> ExecutionPayloadProvider for AlloyExecutionPayloadProvider<P> {
    async fn execution_payload_by_block(
        &self,
        block_id: BlockId,
    ) -> Result<Option<ExecutionPayload>, ExecutionPayloadProviderError> {
        tracing::trace!(target: "scroll::providers", ?block_id, "fetching execution payload");

        let block = self.provider.get_block(block_id).full().await?;
        Ok(block.map(|b| {
            ExecutionPayload::from_block_slow(
                &b.into_consensus().map_transactions(|tx| tx.inner.into_inner()),
            )
            .0
        }))
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use alloy_primitives::B256;
//     use reth_node_builder::{NodeBuilder, NodeConfig};
//     use reth_node_core::args::RpcServerArgs;
//     use reth_rpc_server_types::RpcModuleSelection;
//     use reth_scroll_node::ScrollNode;
//     use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
//     use scroll_network::SCROLL_MAINNET;
//     use std::path::PathBuf;
//
//     #[tokio::test]
//     async fn test_should_get_execution_payload() -> eyre::Result<()> {
//         let chain_spec = (*SCROLL_MAINNET).clone();
//
//         // Create the node config
//         let node_config = NodeConfig::new(chain_spec)
//             .with_rpc({
//                 let mut args = RpcServerArgs::default()
//                     .with_http()
//                     .with_http_api(RpcModuleSelection::Standard);
//                 args
//             })
//             .set_dev(false);
//
//         let node = ScrollNode;
//         let handle = NodeBuilder::new(node_config.clone())
//             .testing_node(exec.clone())
//             .with_types_and_provider::<ScrollNode, BlockchainProvider<_>>()
//             .with_components(
//                 node.components_builder()
//                     .network(rollup_node::ScrollRollupNetworkBuilder::new(node_args)),
//             )
//             .with_add_ons(node.add_ons())
//             .launch()
//             .await?;
//         let peer_id = *node.network.peer_id();
//         let node = NodeTestContext::new(node, scroll_payload_attributes).await?;
//
//         Ok(())
//     }
// }
