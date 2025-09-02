use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use reth_network_api::FullNetwork;
use reth_scroll_node::ScrollNetworkPrimitives;
use rollup_node_manager::RollupManagerHandle;
use tokio::sync::{oneshot, Mutex, OnceCell};


pub struct RollupNodeRpcExt<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    /// The rollup manager handle
    handle: tokio::sync::OnceCell<RollupManagerHandle<N>>,
    /// Oneshot channel for receiving the rollup manager handle
    rx: Mutex<Option<oneshot::Receiver<RollupManagerHandle<N>>>>,
}

impl<N> RollupNodeRpcExt<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    pub fn new(rx: oneshot::Receiver<RollupManagerHandle<N>>) -> Self {
        Self { rx: Mutex::new(Some(rx)), handle: OnceCell::new() }
    }

    async fn rollup_manager_handle(&self) -> eyre::Result<&RollupManagerHandle<N>> {
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


/// This defines an additional namespace for rollup management functions.
#[cfg_attr(not(test), rpc(server, namespace = "rollupNode"))]
#[cfg_attr(test, rpc(server, client, namespace = "rollupNode"))]
pub trait RollupNodeExtApi {
    /// Enable automatic sequencing
    #[method(name = "enableAutomaticSequencing")]
    fn enable_automatic_sequencing(&self) -> RpcResult<bool>;

    /// Disable automatic sequencing
    #[method(name = "disableAutomaticSequencing")]
    fn disable_automatic_sequencing(&self) -> RpcResult<bool>;
}

impl<N> RollupNodeExtApiServer for RollupNodeRpcExt<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    fn enable_automatic_sequencing(&self) -> RpcResult<bool> {
        Ok(true)
    }

    fn disable_automatic_sequencing(&self) -> RpcResult<bool> {
        Ok(true)
    }
}
