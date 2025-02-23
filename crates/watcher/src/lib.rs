//! L1 watcher for the Scroll Rollup Node.

mod error;
#[cfg(any(test, feature = "test-utils"))]
/// Common test helpers
pub mod test_utils;

use alloy_network::Ethereum;
use alloy_primitives::BlockNumber;
use alloy_provider::{Network, Provider};
use alloy_rpc_types_eth::{BlockNumberOrTag, BlockTransactionsKind, Filter, Log};
use error::L1WatcherResult;
use rollup_node_primitives::BlockInfo;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;

/// The block range used to fetch L1 logs.
/// TODO(greg): evaluate the performance using various block ranges.
pub const LOGS_QUERY_BLOCK_RANGE: u64 = 1000;
/// The maximum count of unfinalized blocks we can have in Ethereum.
pub const MAX_UNFINALIZED_BLOCK_COUNT: usize = 96;

/// The main loop interval when L1 watcher is syncing to the tip of the L1.
pub const FAST_SYNC_INTERVAL: Duration = Duration::from_millis(100);
/// The main loop interval when L1 watcher is synced to the tip of the L1.
pub const SLOW_SYNC_INTERVAL: Duration = Duration::from_secs(2);

/// The Ethereum L1 block response.
pub type Block = <Ethereum as Network>::BlockResponse;

/// The Ethereum L1 header response.
pub type Header = <Ethereum as Network>::HeaderResponse;

/// The fork choice state of the L1.
#[derive(Debug, Default, Clone)]
pub struct L1ForkchoiceState {
    head: BlockInfo,
    finalized: BlockInfo,
}

/// The L1 watcher indexes L1 blocks, applying a first level of filtering via log filters.
#[derive(Debug)]
pub struct L1Watcher<EP> {
    /// The L1 execution node provider. The provider should implement some backoff strategy using
    /// [`alloy_transport::layers::RetryBackoffLayer`] in the client in order to avoid excessive
    /// queries on the RPC provider.
    execution_provider: EP,
    /// The buffered unfinalized chain of blocks. Used to detect reorgs of the L1.
    unfinalized_blocks: VecDeque<Header>,
    /// The L1 state info relevant to the rollup node.
    forkchoice_state: L1ForkchoiceState,
    /// The latest indexed block.
    current_block_number: BlockNumber,
    /// The sender part of the channel for [`L1Notification`].
    sender: mpsc::Sender<Arc<L1Notification>>,
    /// The log filter used to index L1 blocks.
    filter: Filter,
}

/// The L1 notification type yielded by the [`L1Watcher`].
#[derive(Debug)]
pub enum L1Notification {
    /// A notification for a reorg of the L1.
    Reorg(BTreeMap<BlockNumber, Header>),
    /// A notification of a new block of interest for the rollup node.
    Block(Block),
    /// A forkchoice update of the L1.
    ForkchoiceState(L1ForkchoiceState),
}

impl<EP> L1Watcher<EP>
where
    EP: Provider + 'static,
{
    /// Spawn a new [`L1Watcher`], starting at start_block. The watcher will iterate the L1,
    /// returning [`L1Notification`] in the returned channel.
    pub async fn spawn(
        execution_provider: EP,
        start_block: BlockNumber,
        filter: Filter,
    ) -> mpsc::Receiver<Arc<L1Notification>> {
        let (tx, rx) = mpsc::channel(LOGS_QUERY_BLOCK_RANGE as usize);

        let fetch_block_info = async |tag: BlockNumberOrTag| {
            let block = loop {
                match execution_provider.get_block(tag.into(), BlockTransactionsKind::Hashes).await
                {
                    Err(err) => {
                        tracing::error!(target: "scroll::watcher", ?err, "failed to fetch {tag} block")
                    }
                    Ok(Some(block)) => break block,
                    _ => unreachable!("should always be a {tag} block"),
                }
            };
            BlockInfo { number: block.header.number, hash: block.header.hash }
        };

        let forkchoice_state = L1ForkchoiceState {
            head: fetch_block_info(BlockNumberOrTag::Latest).await,
            finalized: fetch_block_info(BlockNumberOrTag::Finalized).await,
        };

        let watcher = L1Watcher {
            execution_provider,
            unfinalized_blocks: VecDeque::with_capacity(MAX_UNFINALIZED_BLOCK_COUNT),
            current_block_number: start_block - 1,
            forkchoice_state,
            sender: tx,
            filter,
        };
        tokio::spawn(watcher.run());

        rx
    }

    /// Main execution loop for the [`L1Watcher`].
    pub async fn run(mut self) {
        let mut loop_interval = FAST_SYNC_INTERVAL;
        loop {
            // step the watcher.
            let _ = self
                .step()
                .await
                .inspect_err(|err| tracing::error!(target: "scroll::watcher", ?err));

            // update loop interval if needed.
            if let Ok(synced) = self.is_synced().await {
                loop_interval = if synced { SLOW_SYNC_INTERVAL } else { FAST_SYNC_INTERVAL };
            }

            // sleep the appropriate amount of time.
            tokio::time::sleep(loop_interval).await;
        }
    }

    /// A step of work for the [`L1Watcher`].
    pub async fn step(&mut self) -> L1WatcherResult<()> {
        let finalized = self.finalized_block(false).await?;
        // let position =
        //     self.unfinalized_blocks.drain()

        Ok(())
    }

    /// Handle the latest block, either by adding it to
    /// [`unfinalized_blocks`](field@L1Watcher::unfinalized_blocks) if it extends the chain or
    /// detecting the reorg and emitting a reorg notification through the channel.
    async fn handle_latest_block(&mut self) -> L1WatcherResult<()> {
        let latest = self.latest_block(false).await?;
        self.forkchoice_state.head = BlockInfo::new(latest.header.number, latest.header.hash);

        // shortcircuit if self.unfinalized_blocks is empty
        if self.unfinalized_blocks.is_empty() {
            self.unfinalized_blocks.push_back(latest.header);
            return Ok(())
        }

        let tail_block = self.unfinalized_blocks.back();

        if tail_block.map_or(false, |last| last.hash != latest.header.hash) {
            // if true, we either need to extend self.unfinalized_blocks
            if tail_block.expect("tail block exists").hash == latest.header.parent_hash {
                self.unfinalized_blocks.push_back(latest.header);
                return Ok(())
            }

            // or we have found a reorg
            let reorg_position = self
                .unfinalized_blocks
                .iter()
                .position(|h| h.hash == latest.header.parent_hash)
                .map(|pos| pos + 1)
                .unwrap_or(0);
            let reorged = self
                .unfinalized_blocks
                .drain(reorg_position..)
                .map(|h| (h.number, h))
                .collect::<BTreeMap<_, _>>();
            let _ = self.sender.send(Arc::new(L1Notification::Reorg(reorged))).await;

            self.unfinalized_blocks.push_back(latest.header);
        }

        Ok(())
    }

    /// Returns true if the [`L1Watcher`] is synced to the head of the L1.
    async fn is_synced(&self) -> L1WatcherResult<bool> {
        Ok(self.current_block_number == self.execution_provider.get_block_number().await?)
    }

    /// Returns the latest L1 block.
    async fn latest_block(&self, full: bool) -> L1WatcherResult<Block> {
        Ok(self
            .execution_provider
            .get_block(BlockNumberOrTag::Latest.into(), full.into())
            .await?
            .expect("latest block should always exist"))
    }

    /// Returns the finalized L1 block.
    async fn finalized_block(&self, full: bool) -> L1WatcherResult<Block> {
        Ok(self
            .execution_provider
            .get_block(BlockNumberOrTag::Finalized.into(), full.into())
            .await?
            .expect("finalized block should always exist"))
    }

    /// Returns the next range of logs, using the filter provider in
    /// [`L1Watcher`](field@L1Watcher::filter), for the block range in
    /// \[[`current_block`](field@WatcherSyncStatus::current_block);
    /// [`current_block`](field@WatcherSyncStatus::current_block) + [`LOGS_QUERY_BLOCK_RANGE`]\]
    async fn next_filtered_logs(&self) -> L1WatcherResult<Vec<Log>> {
        // set the block range for the query
        let mut filter = self.filter.clone();
        filter = filter
            .from_block(self.current_block_number)
            .to_block(self.current_block_number + LOGS_QUERY_BLOCK_RANGE);

        Ok(self.execution_provider.get_logs(&filter).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::MockProvider;
    use alloy_primitives::{
        private::arbitrary::{Arbitrary, Unstructured},
        Sealable, B256,
    };
    use alloy_rpc_types_eth::BlockTransactions;
    use rand::RngCore;

    fn test_l1_watcher(
        unfinalized_blocks: VecDeque<Header>,
    ) -> (L1Watcher<MockProvider>, mpsc::Sender<Block>, mpsc::Receiver<Arc<L1Notification>>) {
        let (tx_provider, rx) = mpsc::channel(LOGS_QUERY_BLOCK_RANGE as usize);
        let provider = MockProvider::new(rx);
        let null_block_info = BlockInfo { number: 0, hash: B256::ZERO };

        let (tx, rx) = mpsc::channel(LOGS_QUERY_BLOCK_RANGE as usize);
        (
            L1Watcher {
                execution_provider: provider,
                unfinalized_blocks,
                forkchoice_state: L1ForkchoiceState {
                    head: null_block_info,
                    finalized: null_block_info,
                },
                current_block_number: 0,
                sender: tx,
                filter: Default::default(),
            },
            tx_provider,
            rx,
        )
    }

    fn random_block() -> Block {
        let mut bytes = [0u8; 1000];
        rand::rng().fill_bytes(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);
        let block = alloy_consensus::Block::<()>::arbitrary(&mut u).expect("arbitrary block");
        Block {
            header: Header::from_consensus(block.header.seal_slow(), None, None),
            ..Default::default()
        }
    }

    fn test_chain(len: usize) -> (VecDeque<Header>, Block) {
        let mut headers = VecDeque::with_capacity(len);
        let mut bytes = [0u8; 1000];
        rand::rng().fill_bytes(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let mut parent_hash = u.arbitrary().unwrap();
        let mut number = u.arbitrary().unwrap();
        for _ in 0..len {
            let mut header = alloy_consensus::Header::arbitrary(&mut u).expect("arbitrary header");
            header.parent_hash = parent_hash;
            header.number = number;

            let header = Header::from_consensus(header.seal_slow(), None, None);
            parent_hash = header.hash;
            number += 1;
            headers.push_back(header);
        }
        let mut tip = random_block();
        tip.header.parent_hash = parent_hash;
        tip.header.number = number;
        (headers, tip)
    }

    #[tokio::test]
    async fn test_handle_latest_block_empty_unfinalized() -> eyre::Result<()> {
        let (chain, block) = test_chain(0);
        let (mut watcher, tx_block, _) = test_l1_watcher(chain);
        tx_block.send(block.clone()).await?;

        watcher.handle_latest_block().await?;
        assert_eq!(watcher.unfinalized_blocks.len(), 1);
        assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);

        assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
        assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_latest_block_match_tail() -> eyre::Result<()> {
        let (chain, _) = test_chain(10);
        let block = Block {
            header: chain.back().unwrap().clone(),
            uncles: vec![],
            transactions: BlockTransactions::Hashes(vec![]),
            withdrawals: None,
        };
        let (mut watcher, tx_block, _) = test_l1_watcher(chain);
        tx_block.send(block.clone()).await?;

        watcher.handle_latest_block().await?;
        assert_eq!(watcher.unfinalized_blocks.len(), 10);
        assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);

        assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
        assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_latest_block_not_empty_unfinalized() -> eyre::Result<()> {
        let (chain, block) = test_chain(10);
        let (mut watcher, tx_block, _) = test_l1_watcher(chain);
        tx_block.send(block.clone()).await?;

        watcher.handle_latest_block().await?;
        assert_eq!(watcher.unfinalized_blocks.len(), 11);
        assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);

        assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
        assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_latest_block_reorg_mid() -> eyre::Result<()> {
        let (chain, _) = test_chain(10);
        let block = Block {
            header: chain.get(5).unwrap().clone(),
            uncles: vec![],
            transactions: BlockTransactions::Hashes(vec![]),
            withdrawals: None,
        };
        let (mut watcher, tx_block, _) = test_l1_watcher(chain);
        tx_block.send(block.clone()).await?;

        watcher.handle_latest_block().await?;
        assert_eq!(watcher.unfinalized_blocks.len(), 6);
        assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);

        assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
        assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_latest_block_reorg_all() -> eyre::Result<()> {
        let (chain, _) = test_chain(10);
        let block = random_block();
        let (mut watcher, tx_block, mut notifications) = test_l1_watcher(chain.clone());
        tx_block.send(block.clone()).await?;

        watcher.handle_latest_block().await?;
        assert_eq!(watcher.unfinalized_blocks.len(), 1);
        assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);

        assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
        assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);

        let reorg = notifications.recv().await.unwrap();
        if let L1Notification::Reorg(reorg) = reorg.as_ref() {
            assert_eq!(reorg, &BTreeMap::from_iter(chain.into_iter().map(|b| (b.number, b))));
        } else {
            panic!("Expected reorg notification");
        }

        Ok(())
    }
}
