//! L1 watcher for the Scroll Rollup Node.

pub use constants::{
    L1_MESSAGE_QUEUE_CONTRACT_ADDRESS, L1_WATCHER_LOG_FILTER, ROLLUP_CONTRACT_ADDRESS,
};
mod constants;
mod contract;

pub use error::{EthRequestError, FilterLogError, L1WatcherError};
mod error;

#[cfg(any(test, feature = "test-utils"))]
/// Common test helpers
pub mod test_utils;

use crate::contract::{try_decode_commit_call, try_decode_log, CommitBatch, QueueTransaction};
use std::{collections::VecDeque, sync::Arc, time::Duration};

use alloy_network::Ethereum;
use alloy_primitives::{BlockNumber, Bytes};
use alloy_provider::{Network, Provider};
use alloy_rpc_types_eth::{BlockNumberOrTag, BlockTransactionsKind, Log, TransactionTrait};
use error::L1WatcherResult;
use rollup_node_primitives::{BatchInput, BatchInputBuilder, L1MessageWithBlockNumber};
use scroll_alloy_consensus::TxL1Message;
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

/// The state of the L1.
#[derive(Debug, Default, Clone)]
pub struct L1State {
    head: u64,
    finalized: u64,
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
    l1_state: L1State,
    /// The latest indexed block.
    current_block_number: BlockNumber,
    /// The sender part of the channel for [`L1Notification`].
    sender: mpsc::Sender<Arc<L1Notification>>,
}

/// The L1 notification type yielded by the [`L1Watcher`].
#[derive(Debug)]
pub enum L1Notification {
    /// A notification for a reorg of the L1 up to a given block number.
    Reorg(u64),
    /// A notification of a new batch being commited for the rollup.
    BatchCommit(BatchInput),
    /// A new [`L1Message`] has been added to the L1 message queue.
    L1Message(L1MessageWithBlockNumber),
    /// A new block has been added to the L1.
    NewBlock(u64),
    /// A block has been finalized on the L1.
    Finalized(u64),
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
    ) -> mpsc::Receiver<Arc<L1Notification>> {
        let (tx, rx) = mpsc::channel(LOGS_QUERY_BLOCK_RANGE as usize);

        let fetch_block_number = async |tag: BlockNumberOrTag| {
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
            block.header.number
        };

        let l1_state = L1State {
            head: fetch_block_number(BlockNumberOrTag::Latest).await,
            finalized: fetch_block_number(BlockNumberOrTag::Finalized).await,
        };

        let watcher = L1Watcher {
            execution_provider,
            unfinalized_blocks: VecDeque::with_capacity(MAX_UNFINALIZED_BLOCK_COUNT),
            current_block_number: start_block - 1,
            l1_state,
            sender: tx,
        };
        tokio::spawn(watcher.run());

        rx
    }

    /// Main execution loop for the [`L1Watcher`].
    pub async fn run(mut self) {
        loop {
            // step the watcher.
            let _ = self
                .step()
                .await
                .inspect_err(|err| tracing::error!(target: "scroll::watcher", ?err));

            // update loop interval if needed.
            let loop_interval =
                if self.is_synced() { SLOW_SYNC_INTERVAL } else { FAST_SYNC_INTERVAL };

            // sleep the appropriate amount of time.
            tokio::time::sleep(loop_interval).await;
        }
    }

    /// A step of work for the [`L1Watcher`].
    pub async fn step(&mut self) -> L1WatcherResult<()> {
        // handle the finalized block.
        let finalized = self.finalized_block(false).await?;
        self.handle_finalized_block(&finalized.header);

        // handle the latest block.
        let latest = self.latest_block(false).await?;
        self.handle_latest_block(&finalized.header, &latest.header).await?;

        // index the next range of blocks.
        let logs = self.next_filtered_logs().await?;
        self.handle_l1_messages(&logs).await?;

        // update the latest block the l1 watcher has indexed.
        self.update_current_block(&latest);

        Ok(())
    }

    /// Handle the finalized block:
    ///   - Update state and notify channel about finalization.
    ///   - Drain finalized blocks from state.
    fn handle_finalized_block(&mut self, finalized: &Header) {
        // update the state and notify on channel.
        if self.l1_state.finalized < finalized.number {
            self.l1_state.finalized = finalized.number;
            let _ = self.sender.send(Arc::new(L1Notification::Finalized(finalized.number)));
        }

        // shortcircuit.
        if self.unfinalized_blocks.is_empty() {
            return
        }

        let tail_block = self.unfinalized_blocks.back().expect("tail exists");
        if tail_block.number < finalized.number {
            // drain all, the finalized block is past the tail.
            let _ = self.unfinalized_blocks.drain(0..);
            return
        }

        let finalized_block_position =
            self.unfinalized_blocks.iter().position(|header| header.hash == finalized.hash);

        // drain all blocks up to and including the finalized block.
        if let Some(position) = finalized_block_position {
            self.unfinalized_blocks.drain(0..=position);
        }
    }

    /// Handle the latest block:
    ///   - Skip if latest matches last unfinalized block.
    ///   - Add to unfinalized blocks if it extends the chain.
    ///   - Fetch chain of unfinalized blocks and emit potential reorg otherwise.
    ///   - Finally, update state and notify channel about latest block.
    async fn handle_latest_block(
        &mut self,
        finalized: &Header,
        latest: &Header,
    ) -> L1WatcherResult<()> {
        let tail = self.unfinalized_blocks.back();

        if tail.map_or(false, |h| h.hash == latest.hash) {
            return Ok(())
        } else if tail.map_or(false, |h| h.hash == latest.parent_hash) {
            // latest block extends the tip.
            self.unfinalized_blocks.push_back(latest.clone());
        } else {
            // chain reorged or need to backfill.
            let chain = self.fetch_unfinalized_chain(finalized, latest).await?;

            let reorg_block_number = self
                .unfinalized_blocks
                .iter()
                .zip(chain.iter())
                .find(|(old, new)| old.hash != new.hash)
                .map(|(old, _)| old.number - 1);

            if let Some(number) = reorg_block_number {
                // reset the current block number to the reorged block number if
                // we have indexed passed the reorg.
                if number < self.current_block_number {
                    self.current_block_number = number;
                }

                // send the reorg block number on the channel.
                let _ = self.sender.send(Arc::new(L1Notification::Reorg(number))).await;
            }

            // set the unfinalized chain.
            self.unfinalized_blocks = chain.into();
        }

        // Update the state and notify on the channel.
        self.l1_state.head = latest.number;
        let _ = self.sender.send(Arc::new(L1Notification::NewBlock(latest.number))).await;

        Ok(())
    }

    /// Filters the logs into L1 messages and sends them over the channel.
    async fn handle_l1_messages(&self, logs: &[Log]) -> L1WatcherResult<()> {
        let l1_messages =
            logs.iter().map(|l| (&l.inner, l.block_number)).filter_map(|(log, bn)| {
                try_decode_log::<QueueTransaction>(log)
                    .map(|log| (Into::<TxL1Message>::into(log.data), bn))
            });

        for (msg, bn) in l1_messages {
            let block_number = bn.ok_or(FilterLogError::MissingBlockNumber)?;
            let notification = L1MessageWithBlockNumber::new(block_number, msg);
            let _ = self.sender.send(Arc::new(L1Notification::L1Message(notification))).await;
        }
        Ok(())
    }

    /// Handles the batch commits events.
    async fn handle_batch_commits(&self, logs: &[Log]) -> L1WatcherResult<()> {
        // filter commit logs.
        let commit_tx_hashes =
            logs.iter().map(|l| (l, l.transaction_hash)).filter_map(|(log, tx_hash)| {
                try_decode_log::<CommitBatch>(&log.inner)
                    .map(|decoded| (log, decoded.data, tx_hash))
            });

        for (raw_log, decoded_log, maybe_tx_hash) in commit_tx_hashes {
            // fetch the commit transaction.
            let tx_hash = maybe_tx_hash.ok_or(FilterLogError::MissingTransactionHash)?;
            let transaction = self
                .execution_provider
                .get_transaction_by_hash(tx_hash)
                .await?
                .ok_or(EthRequestError::MissingTransactionHash(tx_hash))?;

            // decode the transaction's input into a commit batch call.
            let commit_info = try_decode_commit_call(transaction.inner.input());
            if let Some(info) = commit_info {
                let batch_index: u64 = decoded_log.batchIndex.saturating_to();
                let block_number =
                    raw_log.block_number.ok_or(FilterLogError::MissingBlockNumber)?;
                let batch_hash = decoded_log.batchHash;
                let blob_hashes = transaction.blob_versioned_hashes().map(|blobs| blobs.to_vec());

                // feed all batch information to the batch input builder.
                let batch_builder = BatchInputBuilder::default()
                    .with_version(info.version())
                    .with_batch_index(batch_index)
                    .with_batch_hash(batch_hash)
                    .with_block_number(block_number)
                    .with_parent_batch_header(info.parent_batch_header())
                    .with_skipped_l1_message_bitmap(info.skipped_l1_message_bitmap())
                    .with_blob_hashes(blob_hashes);

                // if builder can build a batch input from data, notify via channel.
                if let Some(batch_input) = batch_builder.try_build() {
                    let _ =
                        self.sender.send(Arc::new(L1Notification::BatchCommit(batch_input))).await;
                }
            }
        }
        Ok(())
    }

    // TODO: handle finalized log.

    /// Fetches the chain of unfinalized blocks up to and including the latest block, ensuring no
    /// gaps are present in the chain.
    async fn fetch_unfinalized_chain(
        &self,
        finalized: &Header,
        latest: &Header,
    ) -> L1WatcherResult<Vec<Header>> {
        let mut current_block = latest.clone();
        let mut chain = vec![current_block.clone()];

        // loop until we find a block contained in the chain or connected to finalized.
        let mut chain = loop {
            if self.unfinalized_blocks.contains(&current_block) ||
                current_block.parent_hash == finalized.hash
            {
                break chain;
            }

            let block = self
                .execution_provider
                .get_block((current_block.number - 1).into(), BlockTransactionsKind::Hashes)
                .await?
                .ok_or(EthRequestError::MissingBlock(current_block.number - 1))?;
            chain.push(block.header.clone());
            current_block = block.header;
        };

        // order new chain from lowest to highest block number.
        chain.reverse();

        // combine with the available unfinalized blocks.
        let head = chain.first().expect("at least one block");
        let split_position =
            self.unfinalized_blocks.iter().position(|h| h.hash == head.hash).unwrap_or(0);
        let mut prefix = Vec::with_capacity(MAX_UNFINALIZED_BLOCK_COUNT);
        prefix.extend(self.unfinalized_blocks.iter().take(split_position).cloned());
        prefix.append(&mut chain);

        Ok(prefix)
    }

    /// Returns true if the [`L1Watcher`] is synced to the head of the L1.
    fn is_synced(&self) -> bool {
        self.current_block_number == self.l1_state.head
    }

    /// Updates the current block number, saturating at the head of the chain.
    fn update_current_block(&mut self, latest: &Block) {
        let latest_block_number = latest.header.number;
        let current_block_number = self.current_block_number + LOGS_QUERY_BLOCK_RANGE;
        self.current_block_number = if current_block_number > latest_block_number {
            latest_block_number
        } else {
            current_block_number
        };
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
        let mut filter = L1_WATCHER_LOG_FILTER.clone();
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
        Sealable,
    };
    use rand::RngCore;

    fn test_l1_watcher(
        unfinalized_blocks: Vec<Header>,
        provider_blocks: Vec<Header>,
        finalized: Header,
        latest: Header,
    ) -> (L1Watcher<MockProvider>, mpsc::Receiver<Arc<L1Notification>>) {
        let provider_blocks =
            provider_blocks.into_iter().map(|h| Block { header: h, ..Default::default() });
        let finalized = Block { header: finalized, ..Default::default() };
        let latest = Block { header: latest, ..Default::default() };
        let provider = MockProvider::new(provider_blocks, finalized, latest);

        let (tx, rx) = mpsc::channel(LOGS_QUERY_BLOCK_RANGE as usize);
        (
            L1Watcher {
                execution_provider: provider,
                unfinalized_blocks: unfinalized_blocks.into(),
                l1_state: L1State { head: 0, finalized: 0 },
                current_block_number: 0,
                sender: tx,
            },
            rx,
        )
    }

    fn random_header() -> Header {
        let mut bytes = [0u8; 1000];
        rand::rng().fill_bytes(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);
        let header = alloy_consensus::Header::arbitrary(&mut u).expect("arbitrary block");
        Header::from_consensus(header.seal_slow(), None, None)
    }

    fn test_chain(len: usize) -> (Header, Header, Vec<Header>) {
        assert!(len >= 2, "len must be greater than or equal to 2");

        let mut headers = Vec::with_capacity(len);
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
            headers.push(header);
        }
        (headers.first().unwrap().clone(), headers.last().unwrap().clone(), headers)
    }

    fn fork(header: &Header, len: usize) -> Vec<Header> {
        let mut blocks = Vec::with_capacity(len);
        blocks.push(header.clone());

        let next_header = |header: &Header| {
            let mut next = random_header();
            next.parent_hash = header.hash;
            next.number = header.number + 1;
            next
        };
        for i in 0..len - 1 {
            blocks.push(next_header(&blocks[i]));
        }
        blocks
    }

    #[tokio::test]
    async fn test_fetch_unfinalized_chain_no_reorg() -> eyre::Result<()> {
        // Given
        let (finalized, latest, chain) = test_chain(21);
        let unfinalized_blocks = chain[1..11].to_vec();
        let provider_blocks = chain[10..21].to_vec();

        let (watcher, _) = test_l1_watcher(
            unfinalized_blocks,
            provider_blocks.clone(),
            finalized.clone(),
            latest.clone(),
        );

        // When
        let unfinalized_chain = watcher.fetch_unfinalized_chain(&finalized, &latest).await?;

        // Then
        assert_eq!(unfinalized_chain, chain[1..].to_vec());

        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_unfinalized_chain_reorg() -> eyre::Result<()> {
        // Given
        let (finalized, _, chain) = test_chain(21);
        let unfinalized_blocks = chain[1..21].to_vec();
        let mut provider_blocks = fork(&chain[10], 10);
        let latest = provider_blocks[9].clone();

        let (watcher, _) = test_l1_watcher(
            unfinalized_blocks,
            provider_blocks.clone(),
            finalized.clone(),
            latest.clone(),
        );

        // When
        let unfinalized_chain = watcher.fetch_unfinalized_chain(&finalized, &latest).await?;

        // Then
        let mut reorged_chain = chain[1..10].to_vec();
        reorged_chain.append(&mut provider_blocks);
        assert_eq!(unfinalized_chain, reorged_chain);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_finalized_empty_state() -> eyre::Result<()> {
        // Given
        let (finalized, latest, _) = test_chain(2);
        let (mut watcher, _) = test_l1_watcher(vec![], vec![], finalized.clone(), latest);

        // When
        watcher.handle_finalized_block(&finalized);

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_finalize_at_mid_state() -> eyre::Result<()> {
        // Given
        let (_, latest, chain) = test_chain(10);
        let finalized = chain[5].clone();
        let (mut watcher, _) = test_l1_watcher(chain, vec![], finalized.clone(), latest);

        // When
        watcher.handle_finalized_block(&finalized);

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 4);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_finalized_at_end_state() -> eyre::Result<()> {
        // Given
        let (_, latest, chain) = test_chain(10);
        let finalized = latest.clone();
        let (mut watcher, _) = test_l1_watcher(chain, vec![], finalized.clone(), latest);

        // When
        watcher.handle_finalized_block(&finalized);

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 0);

        Ok(())
    }

    //
    // #[tokio::test]
    // async fn test_handle_latest_block_match_tail() -> eyre::Result<()> {
    //     let chain = test_chain(10);
    //     let block = Block {
    //         header: chain.back().unwrap().clone(),
    //         uncles: vec![],
    //         transactions: BlockTransactions::Hashes(vec![]),
    //         withdrawals: None,
    //     };
    //     let (mut watcher, _) = test_l1_watcher(chain, VecDeque::from(vec![block.clone()]));
    //
    //     watcher.handle_latest_block().await?;
    //     assert_eq!(watcher.unfinalized_blocks.len(), 10);
    //     assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);
    //
    //     assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
    //     assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);
    //
    //     Ok(())
    // }
    //
    // #[tokio::test]
    // async fn test_handle_latest_block_not_empty_unfinalized() -> eyre::Result<()> {
    //     let chain = test_chain(10);
    //     let (mut watcher, _) = test_l1_watcher(chain, VecDeque::from(vec![block.clone()]));
    //
    //     watcher.handle_latest_block().await?;
    //     assert_eq!(watcher.unfinalized_blocks.len(), 11);
    //     assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);
    //
    //     assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
    //     assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);
    //
    //     Ok(())
    // }
    //
    // #[tokio::test]
    // async fn test_handle_latest_block_reorg_mid() -> eyre::Result<()> {
    //     let chain, _ = test_chain(10);
    //     let block = Block {
    //         header: chain.get(5).unwrap().clone(),
    //         uncles: vec![],
    //         transactions: BlockTransactions::Hashes(vec![]),
    //         withdrawals: None,
    //     };
    //     let (mut watcher, _) = test_l1_watcher(chain, VecDeque::from(vec![block.clone()]));
    //
    //     watcher.handle_latest_block().await?;
    //     assert_eq!(watcher.unfinalized_blocks.len(), 6);
    //     assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);
    //
    //     assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
    //     assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);
    //
    //     Ok(())
    // }
    //
    // #[tokio::test]
    // async fn test_handle_latest_block_reorg_all() -> eyre::Result<()> {
    //     let (chain, _) = test_chain(10);
    //     let block = random_block();
    //     let (mut watcher, mut notifications) =
    //         test_l1_watcher(chain.clone(), VecDeque::from(vec![block.clone()]));
    //
    //     watcher.handle_latest_block().await?;
    //     assert_eq!(watcher.unfinalized_blocks.len(), 1);
    //     assert_eq!(watcher.unfinalized_blocks.pop_back().unwrap(), block.header);
    //
    //     assert_eq!(watcher.forkchoice_state.head.number, block.header.number);
    //     assert_eq!(watcher.forkchoice_state.head.hash, block.header.hash);
    //
    //     let reorg = notifications.recv().await.unwrap();
    //     if let L1Notification::Reorg(reorg) = reorg.as_ref() {
    //         assert_eq!(reorg, &BTreeMap::from_iter(chain.into_iter().map(|b| (b.number, b))));
    //     } else {
    //         panic!("Expected reorg notification");
    //     }
    //
    //     Ok(())
    // }
}
