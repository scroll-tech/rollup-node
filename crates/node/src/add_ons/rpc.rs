use async_trait::async_trait;
use jsonrpsee::{
    core::RpcResult,
    proc_macros::rpc,
    types::{error, ErrorObjectOwned},
};
use reth_network_api::FullNetwork;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_chain_orchestrator::{ChainOrchestratorHandle, ChainOrchestratorStatus};
use rollup_node_primitives::L1MessageEnvelope;
use scroll_db::L1MessageKey;
use tokio::sync::{oneshot, Mutex, OnceCell};

/// RPC extension for rollup node management operations.
///
/// This struct provides custom JSON-RPC namespaces (`rollupNode` and `rollupNodeAdmin`)
/// that expose rollup management functionality to RPC clients. It manages a connection to the
/// rollup manager through a handle that is initialized lazily via a oneshot channel.
///
/// Both `RollupNodeApiServer` and `RollupNodeAdminApiServer` traits are implemented.
/// You can control which APIs to expose by selectively registering them in the RPC modules.
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
    ///
    /// This struct implements both `RollupNodeApiServer` and `RollupNodeAdminApiServer`.
    /// Control which APIs are exposed by selectively registering them when extending RPC modules.
    pub fn new(rx: oneshot::Receiver<ChainOrchestratorHandle<N>>) -> Self {
        Self {
            rx: Mutex::new(Some(rx)),
            handle: OnceCell::new(),
        }
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

/// Defines the `rollupNode` JSON-RPC namespace for read-only operations.
///
/// This trait provides a custom RPC namespace that exposes rollup node status
/// and query functionality to external clients. The namespace is exposed as `rollupNode`.
///
/// # Usage
/// These methods can be called via JSON-RPC using the `rollupNode` namespace:
/// ```json
/// {"jsonrpc": "2.0", "method": "rollupNode_status", "params": [], "id": 1}
/// ```
/// or using cast:
/// ```bash
/// cast rpc rollupNode_status
/// ```
#[rpc(server, client, namespace = "rollupNode")]
pub trait RollupNodeApi {
    /// Returns the current status of the rollup node.
    #[method(name = "status")]
    async fn status(&self) -> RpcResult<ChainOrchestratorStatus>;

    /// Returns the L1 message by index.
    #[method(name = "getL1MessageByIndex")]
    async fn get_l1_message_by_index(&self, index: u64) -> RpcResult<Option<L1MessageEnvelope>>;

    /// Returns the L1 message by key.
    #[method(name = "getL1MessageByKey")]
    async fn get_l1_message_by_key(
        &self,
        l1_message_key: L1MessageKey,
    ) -> RpcResult<Option<L1MessageEnvelope>>;
}

/// Defines the `rollupNodeAdmin` JSON-RPC namespace for administrative operations.
///
/// This trait provides a custom RPC namespace that exposes rollup node administrative
/// functionality to external clients. The namespace is exposed as `rollupNodeAdmin` and
/// requires the `--rpc.rollup-node-admin` flag to be enabled.
///
/// # Usage
/// These methods can be called via JSON-RPC using the `rollupNodeAdmin` namespace:
/// ```json
/// {"jsonrpc": "2.0", "method": "rollupNodeAdmin_enableAutomaticSequencing", "params": [], "id": 1}
/// ```
/// or using cast:
/// ```bash
/// cast rpc rollupNodeAdmin_enableAutomaticSequencing
/// ```
#[rpc(server, client, namespace = "rollupNodeAdmin")]
pub trait RollupNodeAdminApi {
    /// Enables automatic sequencing in the rollup node.
    #[method(name = "enableAutomaticSequencing")]
    async fn enable_automatic_sequencing(&self) -> RpcResult<bool>;

    /// Disables automatic sequencing in the rollup node.
    #[method(name = "disableAutomaticSequencing")]
    async fn disable_automatic_sequencing(&self) -> RpcResult<bool>;
}

#[async_trait]
impl<N> RollupNodeApiServer for RollupNodeRpcExt<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    async fn status(&self) -> RpcResult<ChainOrchestratorStatus> {
        let handle = self.rollup_manager_handle().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to get rollup manager handle: {}", e),
                None::<()>,
            )
        })?;

        handle.status().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to get rollup node status: {}", e),
                None::<()>,
            )
        })
    }

    async fn get_l1_message_by_index(&self, index: u64) -> RpcResult<Option<L1MessageEnvelope>> {
        let handle = self.rollup_manager_handle().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to get rollup manager handle: {}", e),
                None::<()>,
            )
        })?;

        handle.get_l1_message_by_key(L1MessageKey::from_queue_index(index)).await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to get L1 message by index: {}", e),
                None::<()>,
            )
        })
    }

    async fn get_l1_message_by_key(
        &self,
        l1_message_key: L1MessageKey,
    ) -> RpcResult<Option<L1MessageEnvelope>> {
        let handle = self.rollup_manager_handle().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to get rollup manager handle: {}", e),
                None::<()>,
            )
        })?;

        handle.get_l1_message_by_key(l1_message_key).await.map_err(|e| {
            ErrorObjectOwned::owned(
                error::INTERNAL_ERROR_CODE,
                format!("Failed to get L1 message by key: {}", e),
                None::<()>,
            )
        })
    }
}

#[async_trait]
impl<N> RollupNodeAdminApiServer for RollupNodeRpcExt<N>
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

// Implement RollupNodeApiServer for Arc<RollupNodeRpcExt<N>> to allow shared ownership
#[async_trait]
impl<N> RollupNodeApiServer for std::sync::Arc<RollupNodeRpcExt<N>>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    async fn status(&self) -> RpcResult<ChainOrchestratorStatus> {
        (**self).status().await
    }

    async fn get_l1_message_by_index(&self, index: u64) -> RpcResult<Option<L1MessageEnvelope>> {
        (**self).get_l1_message_by_index(index).await
    }

    async fn get_l1_message_by_key(
        &self,
        l1_message_key: L1MessageKey,
    ) -> RpcResult<Option<L1MessageEnvelope>> {
        (**self).get_l1_message_by_key(l1_message_key).await
    }
}

// Implement RollupNodeAdminApiServer for Arc<RollupNodeRpcExt<N>> to allow shared ownership
#[async_trait]
impl<N> RollupNodeAdminApiServer for std::sync::Arc<RollupNodeRpcExt<N>>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    async fn enable_automatic_sequencing(&self) -> RpcResult<bool> {
        (**self).enable_automatic_sequencing().await
    }

    async fn disable_automatic_sequencing(&self) -> RpcResult<bool> {
        (**self).disable_automatic_sequencing().await
    }
}
