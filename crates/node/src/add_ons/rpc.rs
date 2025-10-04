use async_trait::async_trait;
use jsonrpsee::{
    core::RpcResult,
    proc_macros::rpc,
    types::{error, ErrorObjectOwned},
};
use reth_network_api::FullNetwork;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_chain_orchestrator::ChainOrchestratorHandle;
use tokio::sync::{oneshot, Mutex, OnceCell};

/// RPC extension for rollup node management operations.
///
/// This struct provides a custom JSON-RPC namespace (`rollupNode`) that exposes
/// rollup management functionality to RPC clients. It manages a connection to the
/// rollup manager through a handle that is initialized lazily via a oneshot channel.
#[derive(Debug)]
pub struct RollupNodeRpcExt<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    /// Cached rollup manager handle, initialized lazily via `OnceCell`
    handle: tokio::sync::OnceCell<ChainOrchestratorHandle<N>>,
    /// Oneshot channel receiver for obtaining the rollup manager handle during initialization
    rx: Mutex<Option<oneshot::Receiver<ChainOrchestratorHandle<N>>>>,
}

impl<N> RollupNodeRpcExt<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    /// Creates a new RPC extension with a receiver for the rollup manager handle.
    pub fn new(rx: oneshot::Receiver<ChainOrchestratorHandle<N>>) -> Self {
        Self { rx: Mutex::new(Some(rx)), handle: OnceCell::new() }
    }

    /// Gets or initializes the rollup manager handle.
    ///
    /// This method lazily initializes the rollup manager handle by consuming the oneshot
    /// receiver. Subsequent calls will return the cached handle.
    async fn rollup_manager_handle(&self) -> eyre::Result<&ChainOrchestratorHandle<N>> {
        self.handle
            .get_or_try_init(|| async {
                let rx = {
                    let mut g = self.rx.lock().await;
                    g.take().ok_or_else(|| eyre::eyre!("receiver already consumed"))?
                };
                rx.await.map_err(|e| eyre::eyre!("failed to receive handle: {e}"))
            })
            .await
    }
}

/// Defines the `rollupNode` JSON-RPC namespace for rollup management operations.
///
/// This trait provides a custom RPC namespace that exposes rollup node management
/// functionality to external clients. The namespace is exposed as `rollupNode` and
/// provides methods for controlling automatic sequencing behavior.
///
/// # Usage
/// These methods can be called via JSON-RPC using the `rollupNode` namespace:
/// ```json
/// {"jsonrpc": "2.0", "method": "rollupNode_enableAutomaticSequencing", "params": [], "id": 1}
/// ```
/// or using cast:
/// ```bash
/// cast rpc rollupNode_enableAutomaticSequencing
/// ```
#[rpc(server, client, namespace = "rollupNode")]
pub trait RollupNodeExtApi {
    /// Enables automatic sequencing in the rollup node.
    #[method(name = "enableAutomaticSequencing")]
    async fn enable_automatic_sequencing(&self) -> RpcResult<bool>;

    /// Disables automatic sequencing in the rollup node.
    #[method(name = "disableAutomaticSequencing")]
    async fn disable_automatic_sequencing(&self) -> RpcResult<bool>;
}

#[async_trait]
impl<N> RollupNodeExtApiServer for RollupNodeRpcExt<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    async fn enable_automatic_sequencing(&self) -> RpcResult<bool> {
        let handle = self.rollup_manager_handle().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to get rollup manager handle: {}", e),
                None::<()>,
            )
        })?;

        handle.enable_automatic_sequencing().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to enable automatic sequencing: {}", e),
                None::<()>,
            )
        })
    }

    async fn disable_automatic_sequencing(&self) -> RpcResult<bool> {
        let handle = self.rollup_manager_handle().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to get rollup manager handle: {}", e),
                None::<()>,
            )
        })?;

        handle.disable_automatic_sequencing().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to disable automatic sequencing: {}", e),
                None::<()>,
            )
        })
    }
}
