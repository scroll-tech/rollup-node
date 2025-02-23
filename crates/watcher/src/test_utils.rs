use crate::Block;
use std::sync::Arc;

use alloy_network::Ethereum;
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types_eth::{BlockId, BlockTransactionsKind};
use alloy_transport::{BoxTransport, TransportResult};
use tokio::sync::{mpsc, Mutex};

/// A mock implementation of the [`Provider`] trait.
#[derive(Debug)]
pub struct MockProvider {
    blocks: Arc<Mutex<mpsc::Receiver<Block>>>,
}

impl MockProvider {
    /// Returns a new [`MockProvider`] from a receiver for [`Block`].
    pub fn new(rx: mpsc::Receiver<Block>) -> Self {
        Self { blocks: Arc::new(Mutex::new(rx)) }
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn root(&self) -> &RootProvider<BoxTransport, Ethereum> {
        unreachable!("unused calls")
    }

    async fn get_block(
        &self,
        _block_id: BlockId,
        _kind: BlockTransactionsKind,
    ) -> TransportResult<Option<Block>> {
        Ok(Some(self.blocks.lock().await.recv().await.expect("missing block in channel")))
    }
}
