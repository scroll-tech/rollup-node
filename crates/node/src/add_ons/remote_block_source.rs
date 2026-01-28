//! Remote block source add-on for importing blocks from a remote L2 node
//! and building new blocks on top.

use crate::args::RemoteBlockSourceArgs;
use alloy_primitives::Signature;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_client::RpcClient;
use alloy_transport::layers::RetryBackoffLayer;
use futures::StreamExt;
use reth_network_api::{FullNetwork, PeerId};
use reth_scroll_node::ScrollNetworkPrimitives;
use reth_tasks::shutdown::Shutdown;
use reth_tokio_util::EventStream;
use rollup_node_chain_orchestrator::{ChainOrchestratorEvent, ChainOrchestratorHandle};
use scroll_alloy_network::Scroll;
use scroll_network::NewBlockWithPeer;
use tokio::time::{interval, Duration};

/// Remote block source add-on that imports blocks from a trusted remote L2 node
/// and triggers block building on top of each imported block.
#[derive(Debug)]
pub struct RemoteBlockSourceAddOn<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives>,
{
    /// Configuration for the remote block source.
    config: RemoteBlockSourceArgs,
    /// Handle to the chain orchestrator for sending commands.
    handle: ChainOrchestratorHandle<N>,
    /// Tracks the last block number we imported from remote.
    /// This is different from local head because we build blocks on top of imports.
    last_imported_block: u64,
}

impl<N> RemoteBlockSourceAddOn<N>
where
    N: FullNetwork<Primitives = ScrollNetworkPrimitives> + Send + Sync + 'static,
{
    /// Creates a new remote block source add-on.
    pub async fn new(
        config: RemoteBlockSourceArgs,
        handle: ChainOrchestratorHandle<N>,
    ) -> eyre::Result<Self> {
        let last_imported_block = handle.status().await?.l2.fcs.head_block_info().number;
        Ok(Self { config, handle, last_imported_block })
    }

    /// Runs the remote block source until shutdown.
    pub async fn run_until_shutdown(mut self, mut shutdown: Shutdown) -> eyre::Result<()> {
        let Some(url) = self.config.url.clone() else {
            tracing::error!(target: "scroll::remote_source", "URL required when remote-source is enabled");
            return Err(eyre::eyre!("URL required when remote-source is enabled"));
        };

        // Build remote provider with retry layer
        let retry_layer = RetryBackoffLayer::new(10, 100, 330);
        let client = RpcClient::builder().layer(retry_layer).http(url);
        let remote = ProviderBuilder::<_, _, Scroll>::default().connect_client(client);

        // Get event listener for waiting on block completion
        let mut event_stream = match self.handle.get_event_listener().await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!(target: "scroll::remote_source", ?e, "Failed to get event listener");
                return Err(eyre::eyre!(e));
            }
        };

        let mut poll_interval = interval(Duration::from_millis(self.config.poll_interval_ms));

        loop {
            tokio::select! {
                biased;
                _guard = &mut shutdown => break,
                _ = poll_interval.tick() => {
                    if let Err(e) = self.follow_and_build(&remote, &mut event_stream).await {
                        tracing::error!(target: "scroll::remote_source", ?e, "Sync error");
                    }
                }
            }
        }

        Ok(())
    }

    /// Follows the remote node and builds blocks on top of imported blocks.
    async fn follow_and_build<P: Provider<Scroll>>(
        &mut self,
        remote: &P,
        event_stream: &mut EventStream<ChainOrchestratorEvent>,
    ) -> eyre::Result<()> {
        loop {
            // Get remote head
            let remote_block = remote
                .get_block_by_number(alloy_eips::BlockNumberOrTag::Latest)
                .full()
                .await?
                .ok_or_else(|| eyre::eyre!("Remote block not found"))?;

            let remote_head = remote_block.header.number;

            // Compare against last imported block
            if remote_head <= self.last_imported_block {
                tracing::trace!(target: "scroll::remote_source",
                    last_imported = self.last_imported_block,
                    remote_head,
                    "Already synced with remote");
                return Ok(());
            }

            let blocks_behind = remote_head - self.last_imported_block;
            tracing::info!(target: "scroll::remote_source",
                last_imported = self.last_imported_block,
                remote_head,
                blocks_behind,
                "Catching up");

            // Fetch and import the next block from remote
            let next_block_num = self.last_imported_block + 1;
            let block = remote
                .get_block_by_number(next_block_num.into())
                .full()
                .await?
                .ok_or_else(|| eyre::eyre!("Block {} not found", next_block_num))?
                .into_consensus()
                .map_transactions(|tx| tx.inner.into_inner());

            // Create NewBlockWithPeer with dummy peer_id and signature (trusted source)
            let block_with_peer = NewBlockWithPeer {
                peer_id: PeerId::default(),
                block,
                signature: Signature::new(Default::default(), Default::default(), false),
            };

            // Import the block (this will cause a reorg if we had a locally built block at this
            // height)
            let chain_import = match self.handle.import_block(block_with_peer).await {
                Ok(Ok(chain_import)) => {
                    self.last_imported_block = next_block_num;
                    chain_import
                }
                Ok(Err(e)) => {
                    return Err(eyre::eyre!("Import block failed: {}", e));
                }
                Err(e) => {
                    return Err(eyre::eyre!("chain orchestrator command channel error: {}", e));
                }
            };

            if !chain_import.result.is_valid() {
                tracing::info!(target: "scroll::remote_source",
                    result = ?chain_import.result,
                    "Imported block is not valid according to forkchoice, skipping build");
                continue;
            }

            // Trigger block building on top of the imported block
            self.handle.build_block();

            // Wait for BlockSequenced event
            tracing::debug!(target: "scroll::remote_source", "Waiting for block to be built...");
            loop {
                match event_stream.next().await {
                    Some(ChainOrchestratorEvent::BlockSequenced(block)) => {
                        tracing::info!(target: "scroll::remote_source",
                            block_number = block.header.number,
                            block_hash = ?block.hash_slow(),
                            "Block built successfully, proceeding to next");
                        break;
                    }
                    Some(_) => {
                        // Ignore other events, keep waiting
                    }
                    None => {
                        return Err(eyre::eyre!("Event stream ended unexpectedly"));
                    }
                }
            }

            // Loop continues to process next block
        }
    }
}
