//! L1 watcher for the Scroll Rollup Node.

mod error;
pub use error::{EthRequestError, FilterLogError, L1WatcherError};

mod metrics;
pub use metrics::WatcherMetrics;

#[cfg(any(test, feature = "test-utils"))]
/// Common test helpers
pub mod test_utils;

use alloy_network::Ethereum;
use alloy_primitives::{ruint::UintTryTo, BlockNumber, B256};
use alloy_provider::{Network, Provider};
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter, Log, TransactionTrait};
use alloy_sol_types::SolEvent;
use error::L1WatcherResult;
use itertools::Itertools;
use rollup_node_primitives::{BatchCommitData, BoundedVec, ConsensusUpdate, NodeConfig};
use rollup_node_providers::SystemContractProvider;
use scroll_alloy_consensus::TxL1Message;
use scroll_l1::abi::logs::{try_decode_log, CommitBatch, FinalizeBatch, QueueTransaction};
use std::{
    cmp::Ordering,
    fmt::{Debug, Display, Formatter},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;

/// The maximum count of unfinalized blocks we can have in Ethereum.
pub const MAX_UNFINALIZED_BLOCK_COUNT: usize = 96;

/// The main loop interval when L1 watcher is synced to the tip of the L1.
#[cfg(any(test, feature = "test-utils"))]
pub const SLOW_SYNC_INTERVAL: Duration = Duration::from_millis(1);
/// The main loop interval when L1 watcher is synced to the tip of the L1.
#[cfg(not(any(test, feature = "test-utils")))]
pub const SLOW_SYNC_INTERVAL: Duration = Duration::from_secs(2);

/// The maximum amount of retained headers for reorg detection.
#[cfg(any(test, feature = "test-utils"))]
pub const HEADER_CAPACITY: usize = 100 * MAX_UNFINALIZED_BLOCK_COUNT;
/// The maximum amount of retained headers for reorg detection.
#[cfg(not(any(test, feature = "test-utils")))]
pub const HEADER_CAPACITY: usize = 2 * MAX_UNFINALIZED_BLOCK_COUNT;

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
    /// [`alloy_transport::layers::RetryBackoffLayer`], some caching strategy using
    /// [`alloy_provider::layers::CacheProvider`] and some rate limiting policy with
    /// [`alloy_transport::layers::RateLimitRetryPolicy`] in the client/transport in order to avoid
    /// excessive queries on the RPC provider.
    execution_provider: EP,
    /// The buffered unfinalized chain of blocks. Used to detect reorgs of the L1.
    unfinalized_blocks: BoundedVec<Header>,
    /// The L1 state info relevant to the rollup node.
    l1_state: L1State,
    /// The latest indexed block.
    current_block_number: BlockNumber,
    /// The sender part of the channel for [`L1Notification`].
    sender: mpsc::Sender<Arc<L1Notification>>,
    /// The rollup node configuration.
    config: Arc<NodeConfig>,
    /// The metrics for the watcher.
    metrics: WatcherMetrics,
    /// Whether the watcher is synced to the L1 head.
    is_synced: bool,
    /// The log query block range.
    log_query_block_range: u64,
}

/// The L1 notification type yielded by the [`L1Watcher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum L1Notification {
    /// A notification that the L1 watcher has processed up to a given block number.
    Processed(u64),
    /// A notification for a reorg of the L1 up to a given block number.
    Reorg(u64),
    /// A new batch has been committed on the L1 rollup contract.
    BatchCommit(BatchCommitData),
    /// A new batch has been finalized on the L1 rollup contract.
    BatchFinalization {
        /// The hash of the finalized batch.
        hash: B256,
        /// The index of the finalized batch.
        index: u64,
        /// The block number the batch was finalized at.
        block_number: BlockNumber,
    },
    /// A new `L1Message` has been added to the L1 message queue.
    L1Message {
        /// The L1 message.
        message: TxL1Message,
        /// The block number at which the L1 message was emitted.
        block_number: u64,
        /// The timestamp at which the L1 message was emitted.
        block_timestamp: u64,
    },
    /// The consensus config has been updated.
    Consensus(ConsensusUpdate),
    /// A new block has been added to the L1.
    NewBlock(u64),
    /// A block has been finalized on the L1.
    Finalized(u64),
    /// A notification that the L1 watcher is synced to the L1 head.
    Synced,
}

impl Display for L1Notification {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Processed(n) => write!(f, "Processed({n})"),
            Self::Reorg(n) => write!(f, "Reorg({n:?})"),
            Self::BatchCommit(b) => {
                write!(f, "BatchCommit {{ hash: {}, index: {} }}", b.hash, b.index)
            }
            Self::BatchFinalization { hash, index, block_number } => write!(
                f,
                "BatchFinalization{{ hash: {hash}, index: {index}, block_number: {block_number} }}",
            ),
            Self::L1Message { message, block_number, .. } => write!(
                f,
                "L1Message{{ index: {}, block_number: {} }}",
                message.queue_index, block_number
            ),
            Self::Consensus(u) => write!(f, "{u:?}"),
            Self::NewBlock(n) => write!(f, "NewBlock({n})"),
            Self::Finalized(n) => write!(f, "Finalized({n})"),
            Self::Synced => write!(f, "Synced"),
        }
    }
}

impl<EP> L1Watcher<EP>
where
    EP: Provider + SystemContractProvider + 'static,
{
    /// Spawn a new [`L1Watcher`], starting at `start_block`. The watcher will iterate the L1,
    /// returning [`L1Notification`] in the returned channel.
    pub async fn spawn(
        execution_provider: EP,
        start_block: Option<u64>,
        config: Arc<NodeConfig>,
        log_query_block_range: u64,
    ) -> mpsc::Receiver<Arc<L1Notification>> {
        tracing::trace!(target: "scroll::watcher", ?start_block, ?config, "spawning L1 watcher");

        let (tx, rx) = mpsc::channel(log_query_block_range as usize);

        let fetch_block_number = async |tag: BlockNumberOrTag| {
            let block = loop {
                match execution_provider.get_block(tag.into()).await {
                    Err(err) => {
                        tracing::error!(target: "scroll::watcher", ?err, "failed to fetch {tag} block")
                    }
                    Ok(Some(block)) => break block,
                    _ => unreachable!("should always be a {tag} block"),
                }
            };
            block.header.number
        };

        // fetch l1 state.
        let l1_state = L1State {
            head: fetch_block_number(BlockNumberOrTag::Latest).await,
            finalized: fetch_block_number(BlockNumberOrTag::Finalized).await,
        };

        // init the watcher.
        let watcher = Self {
            execution_provider,
            unfinalized_blocks: BoundedVec::new(HEADER_CAPACITY),
            current_block_number: start_block.unwrap_or(config.start_l1_block).saturating_sub(1),
            l1_state,
            sender: tx,
            config,
            metrics: WatcherMetrics::default(),
            is_synced: false,
            log_query_block_range,
        };

        // notify at spawn.
        watcher
            .notify(L1Notification::Finalized(watcher.l1_state.finalized))
            .await
            .expect("channel is open in this context");
        watcher
            .notify(L1Notification::NewBlock(watcher.l1_state.head))
            .await
            .expect("channel is open in this context");

        tokio::spawn(watcher.run());

        rx
    }

    /// Main execution loop for the [`L1Watcher`].
    pub async fn run(mut self) {
        loop {
            // step the watcher.
            if let Err(L1WatcherError::SendError(_)) = self
                .step()
                .await
                .inspect_err(|err| tracing::error!(target: "scroll::watcher", ?err))
            {
                tracing::warn!(target: "scroll::watcher", "L1 watcher channel closed, stopping the watcher");
                break;
            }

            // sleep if we are synced.
            if self.is_synced {
                tokio::time::sleep(SLOW_SYNC_INTERVAL).await;
            } else if self.current_block_number == self.l1_state.head {
                // if we have synced to the head of the L1, notify the channel and set the
                // `is_synced`` flag.
                if let Err(L1WatcherError::SendError(_)) = self.notify(L1Notification::Synced).await
                {
                    tracing::warn!(target: "scroll::watcher", "L1 watcher channel closed, stopping the watcher");
                    break;
                }
                self.is_synced = true;
            }
        }
    }

    /// A step of work for the [`L1Watcher`].
    pub async fn step(&mut self) -> L1WatcherResult<()> {
        // handle the finalized block.
        let finalized = self.finalized_block().await?;
        self.handle_finalized_block(&finalized.header).await?;

        // handle the latest block.
        let latest = self.latest_block().await?;
        self.handle_latest_block(&finalized.header, &latest.header).await?;

        if latest.header.number != self.current_block_number {
            // index the next range of blocks.
            let logs = self.next_filtered_logs(latest.header.number).await?;

            // prepare notifications.
            let mut notifications = Vec::with_capacity(logs.len());

            // handle all events.
            notifications.extend(self.handle_l1_messages(&logs).await?);
            notifications.extend(self.handle_batch_commits(&logs).await?);
            notifications.extend(self.handle_batch_finalization(&logs).await?);
            if let Some(system_contract_update) =
                self.handle_system_contract_update(&latest).await?
            {
                notifications.push(system_contract_update);
            }

            // send all notifications on the channel.
            self.notify_all(notifications).await?;

            // update the latest block the l1 watcher has indexed.
            self.update_current_block(&latest).await?;
        }

        Ok(())
    }

    /// Handle the finalized block:
    ///   - Update state and notify channel about finalization.
    ///   - Drain finalized blocks from state.
    #[tracing::instrument(
        target = "scroll::watcher",
        skip_all,
        fields(curr_finalized = ?self.l1_state.finalized, new_finalized = ?finalized.number)
    )]
    async fn handle_finalized_block(&mut self, finalized: &Header) -> L1WatcherResult<()> {
        // update the state and notify on channel.
        if self.l1_state.finalized < finalized.number {
            tracing::trace!(target: "scroll::watcher", number = finalized.number, hash = ?finalized.hash, "new finalized block");

            self.l1_state.finalized = finalized.number;
            self.notify(L1Notification::Finalized(finalized.number)).await?;
        }

        // shortcircuit.
        if self.unfinalized_blocks.is_empty() {
            tracing::trace!(target: "scroll::watcher", "no unfinalized blocks");
            return Ok(());
        }

        let tail_block = self.unfinalized_blocks.last().expect("tail exists");
        if tail_block.number < finalized.number {
            // clear, the finalized block is past the tail.
            tracing::trace!(target: "scroll::watcher", tail = ?tail_block.number, finalized = ?finalized.number, "draining all unfinalized blocks");
            self.unfinalized_blocks.clear();
            return Ok(());
        }

        let finalized_block_position =
            self.unfinalized_blocks.iter().position(|header| header.hash == finalized.hash);

        // drain all blocks up to and including the finalized block.
        if let Some(position) = finalized_block_position {
            tracing::trace!(target: "scroll::watcher", "draining range {:?}", 0..=position);
            self.unfinalized_blocks.drain(0..=position);
        }

        Ok(())
    }

    /// Handle the latest block:
    ///   - Skip if latest matches last unfinalized block.
    ///   - Add to unfinalized blocks if it extends the chain.
    ///   - Fetch chain of unfinalized blocks and emit potential reorg otherwise.
    ///   - Finally, update state and notify channel about latest block.
    #[tracing::instrument(target = "scroll::watcher", skip_all, fields(latest = ?latest.number))]
    async fn handle_latest_block(
        &mut self,
        finalized: &Header,
        latest: &Header,
    ) -> L1WatcherResult<()> {
        let tail = self.unfinalized_blocks.last();

        if tail.is_some_and(|h| h.hash == latest.hash) {
            return Ok(());
        } else if tail.is_some_and(|h| h.hash == latest.parent_hash) {
            // latest block extends the tip.
            tracing::trace!(target: "scroll::watcher", number = ?latest.number, hash = ?latest.hash, "block extends chain");
            self.unfinalized_blocks.push_back(latest.clone());
        } else {
            // chain reorged or need to backfill.
            tracing::trace!(target: "scroll::watcher", number = ?latest.number, hash = ?latest.hash, "gap or reorg");
            let chain = self.fetch_unfinalized_chain(finalized, latest).await?;

            let reorg_block_number = self
                .unfinalized_blocks
                .iter()
                .zip(chain.iter())
                .find(|(old, new)| old.hash != new.hash)
                .map(|(old, _)| old.number.saturating_sub(1));

            // set the unfinalized chain.
            self.unfinalized_blocks = chain;

            if let Some(number) = reorg_block_number {
                tracing::debug!(target: "scroll::watcher", ?number, "reorg");

                // update metrics.
                self.metrics.reorgs.increment(1);
                self.metrics.reorg_depths.record(self.l1_state.head.saturating_sub(number) as f64);

                // reset the current block number to the reorged block number if
                // we have indexed passed the reorg.
                if number < self.current_block_number {
                    self.current_block_number = number;
                }

                // send the reorg block number on the channel.
                self.notify(L1Notification::Reorg(number)).await?;
            }
        }

        // Update the state and notify on the channel.
        tracing::trace!(target: "scroll::watcher", number = ?latest.number, hash = ?latest.hash, "new block");
        self.l1_state.head = latest.number;
        self.notify(L1Notification::NewBlock(latest.number)).await?;

        Ok(())
    }

    /// Filters the logs into L1 messages and sends them over the channel.
    #[tracing::instrument(skip_all)]
    async fn handle_l1_messages(&self, logs: &[Log]) -> L1WatcherResult<Vec<L1Notification>> {
        let mut l1_messages = logs
            .iter()
            .map(|l| (&l.inner, l.block_number, l.block_timestamp))
            .filter_map(|(log, bn, ts)| {
                try_decode_log::<QueueTransaction>(log)
                    .map(|log| (Into::<TxL1Message>::into(log.data), bn, ts))
            })
            .collect::<Vec<_>>();

        // prepare notifications
        let mut notifications = Vec::with_capacity(l1_messages.len());

        // sort the message by index and group by block number.
        l1_messages.sort_by(|(m1, _, _), (m2, _, _)| m1.queue_index.cmp(&m2.queue_index));
        let groups = l1_messages.into_iter().chunk_by(|(_, bn, _)| *bn);
        let groups: Vec<_> =
            groups.into_iter().map(|(bn, group)| (bn, group.collect::<Vec<_>>())).collect();

        for (bn, group) in groups {
            let block_number = bn.ok_or(FilterLogError::MissingBlockNumber)?;
            // fetch the timestamp if missing from the log.
            let block_timestamp = if let Some(ts) = group.first().and_then(|(_, _, ts)| *ts) {
                ts
            } else {
                self.execution_provider
                    .get_block(block_number.into())
                    .await?
                    .map(|b| b.header.timestamp)
                    .ok_or(FilterLogError::MissingBlockTimestamp)?
            };

            // push notifications in vector.
            for (msg, _, _) in group {
                notifications.push(L1Notification::L1Message {
                    message: msg,
                    block_number,
                    block_timestamp,
                });
            }
        }
        Ok(notifications)
    }

    /// Handles the batch commits events.
    #[tracing::instrument(skip_all)]
    async fn handle_batch_commits(&self, logs: &[Log]) -> L1WatcherResult<Vec<L1Notification>> {
        // filter commit logs.
        let mut commit_logs_with_tx = logs
            .iter()
            .map(|l| (l, l.transaction_hash))
            .filter_map(|(log, tx_hash)| {
                let tx_hash = tx_hash?;
                try_decode_log::<CommitBatch>(&log.inner)
                    .map(|decoded| (log, decoded.data, tx_hash))
            })
            .collect::<Vec<_>>();

        // prepare notifications
        let mut notifications = Vec::with_capacity(commit_logs_with_tx.len());

        // sort the commits by block number then batch index, then group by tx hash.
        commit_logs_with_tx.sort_by(|(log_a, data_a, _), (log_b, data_b, _)| {
            log_a
                .block_number
                .and_then(|a| log_b.block_number.map(|b| a.cmp(&b)))
                .unwrap_or(Ordering::Equal)
                .then_with(|| data_a.batch_index.cmp(&data_b.batch_index))
        });
        let groups = commit_logs_with_tx.into_iter().chunk_by(|(_, _, hash)| *hash);
        let groups: Vec<_> =
            groups.into_iter().map(|(hash, group)| (hash, group.collect::<Vec<_>>())).collect();

        // iterate each group of commits
        for (tx_hash, group) in groups {
            // fetch the commit transaction.
            let transaction = self
                .execution_provider
                .get_transaction_by_hash(tx_hash)
                .await?
                .ok_or(EthRequestError::MissingTransactionHash(tx_hash))?;

            // get the optional blobs and calldata.
            let mut blob_versioned_hashes =
                transaction.blob_versioned_hashes().unwrap_or(&[]).iter().copied();
            let input = Arc::new(transaction.input().clone());

            // iterate the logs emitted in the group
            for (raw_log, decoded_log, _) in group {
                let block_number =
                    raw_log.block_number.ok_or(FilterLogError::MissingBlockNumber)?;
                // if the log is missing the block timestamp, we need to fetch it.
                // the block timestamp is necessary in order to derive the beacon
                // slot and query the blobs.
                let block_timestamp = if let Some(ts) = raw_log.block_timestamp {
                    ts
                } else {
                    self.execution_provider
                        .get_block(block_number.into())
                        .await?
                        .map(|b| b.header.timestamp)
                        .ok_or(FilterLogError::MissingBlockTimestamp)?
                };
                let batch_index =
                    decoded_log.batch_index.uint_try_to().expect("u256 to u64 conversion error");

                // push in vector.
                notifications.push(L1Notification::BatchCommit(BatchCommitData {
                    hash: decoded_log.batch_hash,
                    index: batch_index,
                    block_number,
                    block_timestamp,
                    calldata: input.clone(),
                    blob_versioned_hash: blob_versioned_hashes.next(),
                    finalized_block_number: None,
                }));
            }
        }
        Ok(notifications)
    }

    /// Handles the finalize batch events.
    #[tracing::instrument(skip_all)]
    async fn handle_batch_finalization(
        &self,
        logs: &[Log],
    ) -> L1WatcherResult<Vec<L1Notification>> {
        // filter finalize logs.
        logs.iter()
            .map(|l| (l, l.block_number))
            .filter_map(|(log, bn)| {
                try_decode_log::<FinalizeBatch>(&log.inner).map(|decoded| (decoded.data, bn))
            })
            .map(|(decoded_log, maybe_block_number)| {
                // fetch the finalize transaction.
                let block_number = maybe_block_number.ok_or(FilterLogError::MissingBlockNumber)?;
                let index =
                    decoded_log.batch_index.uint_try_to().expect("u256 to u64 conversion error");
                Ok(L1Notification::BatchFinalization {
                    hash: decoded_log.batch_hash,
                    index,
                    block_number,
                })
            })
            .collect()
    }

    /// Handles the system contract update events.
    /// TODO(greg): update with logs once system contract emits logs.
    async fn handle_system_contract_update(
        &self,
        latest_block: &Block,
    ) -> L1WatcherResult<Option<L1Notification>> {
        // refresh the signer every new block.
        if latest_block.header.number != self.l1_state.head {
            let signer = self
                .execution_provider
                .authorized_signer(self.config.address_book.system_contract_address)
                .await?;
            return Ok(Some(L1Notification::Consensus(ConsensusUpdate::AuthorizedSigner(signer))));
        }

        Ok(None)
    }

    /// Fetches the chain of unfinalized blocks up to and including the latest block, ensuring no
    /// gaps are present in the chain.
    #[tracing::instrument(target = "scroll::watcher", skip_all)]
    async fn fetch_unfinalized_chain(
        &self,
        finalized: &Header,
        latest: &Header,
    ) -> L1WatcherResult<BoundedVec<Header>> {
        let mut current_block = latest.clone();
        let mut chain = vec![current_block.clone()];

        // loop until we find a block contained in the chain, connected to finalized or latest is
        // finalized.
        let (split_position, mut chain) = loop {
            let pos = self.unfinalized_blocks.iter().rposition(|h| h == &current_block);
            if pos.is_some() ||
                current_block.parent_hash == finalized.hash ||
                current_block.hash == finalized.hash
            {
                break (pos, chain);
            }

            tracing::trace!(target: "scroll::watcher", number = ?(current_block.number.saturating_sub(1)), "fetching block");
            let block = self
                .execution_provider
                .get_block((current_block.number.saturating_sub(1)).into())
                .await?
                .ok_or_else(|| {
                    EthRequestError::MissingBlock(current_block.number.saturating_sub(1))
                })?;
            chain.push(block.header.clone());
            current_block = block.header;
        };

        // order new chain from lowest to highest block number.
        chain.reverse();

        // combine with the available unfinalized blocks.
        let split_position = split_position.unwrap_or(0);
        let mut prefix = BoundedVec::new(HEADER_CAPACITY);
        prefix.extend(self.unfinalized_blocks.iter().take(split_position).cloned());
        prefix.extend(chain.into_iter());

        Ok(prefix)
    }

    /// Send all notifications on the channel.
    async fn notify_all(&self, notifications: Vec<L1Notification>) -> L1WatcherResult<()> {
        for notification in notifications {
            self.metrics.process_l1_notification(&notification);
            tracing::trace!(target: "scroll::watcher", %notification, "sending l1 notification");
            self.notify(notification).await?;
        }
        Ok(())
    }

    /// Send the notification in the channel.
    async fn notify(&self, notification: L1Notification) -> L1WatcherResult<()> {
        Ok(self.sender.send(Arc::new(notification)).await.inspect_err(
            |err| tracing::error!(target: "scroll::watcher", ?err, "failed to send notification"),
        )?)
    }

    /// Updates the current block number, saturating at the head of the chain.
    async fn update_current_block(&mut self, latest: &Block) -> L1WatcherResult<()> {
        self.current_block_number = self
            .current_block_number
            .saturating_add(self.log_query_block_range)
            .min(latest.header.number);
        self.notify(L1Notification::Processed(self.current_block_number)).await
    }

    /// Returns the latest L1 block.
    async fn latest_block(&self) -> L1WatcherResult<Block> {
        Ok(self
            .execution_provider
            .get_block(BlockNumberOrTag::Latest.into())
            .await?
            .expect("latest block should always exist"))
    }

    /// Returns the finalized L1 block.
    async fn finalized_block(&self) -> L1WatcherResult<Block> {
        Ok(self
            .execution_provider
            .get_block(BlockNumberOrTag::Finalized.into())
            .await?
            .expect("finalized block should always exist"))
    }

    /// Returns the next range of logs, for the block range in
    /// \[[`current_block`](field@L1Watcher::current_block_number);
    /// [`current_block`](field@L1Watcher::current_block_number) +
    /// [`field@L1Watcher::log_query_block_range`]\].
    async fn next_filtered_logs(&self, latest_block_number: u64) -> L1WatcherResult<Vec<Log>> {
        // set the block range for the query
        let address_book = &self.config.address_book;
        let mut filter = Filter::new()
            .address(vec![
                address_book.rollup_node_contract_address,
                address_book.v1_message_queue_address,
                address_book.v2_message_queue_address,
            ])
            .event_signature(vec![
                QueueTransaction::SIGNATURE_HASH,
                CommitBatch::SIGNATURE_HASH,
                FinalizeBatch::SIGNATURE_HASH,
            ]);
        let to_block = self
            .current_block_number
            .saturating_add(self.log_query_block_range)
            .min(latest_block_number);

        // skip a block for `from_block` since `self.current_block_number` is the last indexed
        // block.
        filter = filter.from_block(self.current_block_number.saturating_add(1)).to_block(to_block);

        tracing::trace!(target: "scroll::watcher", ?filter, "fetching logs");

        Ok(self.execution_provider.get_logs(&filter).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{chain, chain_from, provider::MockProvider};

    use alloy_consensus::{transaction::Recovered, Signed, TxEip1559};
    use alloy_primitives::{Address, U256};
    use alloy_rpc_types_eth::Transaction;
    use alloy_sol_types::{SolCall, SolEvent};
    use arbitrary::Arbitrary;
    use scroll_l1::abi::calls::commitBatchCall;

    const LOG_QUERY_BLOCK_RANGE: u64 = 500;

    // Returns a L1Watcher along with the receiver end of the L1Notifications.
    fn l1_watcher(
        unfinalized_blocks: Vec<Header>,
        provider_blocks: Vec<Header>,
        transactions: Vec<Transaction>,
        finalized: Header,
        latest: Header,
    ) -> (L1Watcher<MockProvider>, mpsc::Receiver<Arc<L1Notification>>) {
        let provider_blocks =
            provider_blocks.into_iter().map(|h| Block { header: h, ..Default::default() });
        let finalized = Block { header: finalized, ..Default::default() };
        let latest = Block { header: latest, ..Default::default() };
        let provider = MockProvider::new(
            provider_blocks,
            transactions.into_iter(),
            std::iter::empty(),
            vec![finalized],
            vec![latest],
        );

        let (tx, rx) = mpsc::channel(LOG_QUERY_BLOCK_RANGE as usize);
        (
            L1Watcher {
                execution_provider: provider,
                unfinalized_blocks: unfinalized_blocks.into(),
                l1_state: L1State { head: 0, finalized: 0 },
                current_block_number: 0,
                sender: tx,
                config: Arc::new(NodeConfig::mainnet()),
                metrics: WatcherMetrics::default(),
                is_synced: false,
                log_query_block_range: LOG_QUERY_BLOCK_RANGE,
            },
            rx,
        )
    }

    #[tokio::test]
    async fn test_should_fetch_unfinalized_chain_without_reorg() -> eyre::Result<()> {
        // Given
        let (finalized, latest, chain) = chain(21);
        let unfinalized_blocks = chain[1..11].to_vec();

        let (watcher, _) = l1_watcher(
            unfinalized_blocks,
            chain.clone(),
            vec![],
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
    async fn test_should_fetch_unfinalized_chain_with_reorg() -> eyre::Result<()> {
        // Given
        let (finalized, _, chain) = chain(21);
        let unfinalized_blocks = chain[1..21].to_vec();
        let mut provider_blocks = chain_from(&chain[10], 10);
        let latest = provider_blocks[9].clone();

        let (watcher, _) = l1_watcher(
            unfinalized_blocks,
            provider_blocks.clone(),
            vec![],
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
    async fn test_should_handle_finalized_with_empty_state() -> eyre::Result<()> {
        // Given
        let (finalized, latest, _) = chain(2);
        let (mut watcher, _rx) = l1_watcher(vec![], vec![], vec![], finalized.clone(), latest);

        // When
        watcher.handle_finalized_block(&finalized).await?;

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_finalize_at_mid_state() -> eyre::Result<()> {
        // Given
        let (_, latest, chain) = chain(10);
        let finalized = chain[5].clone();
        let (mut watcher, _rx) = l1_watcher(chain, vec![], vec![], finalized.clone(), latest);

        // When
        watcher.handle_finalized_block(&finalized).await?;

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 4);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_finalized_at_end_state() -> eyre::Result<()> {
        // Given
        let (_, latest, chain) = chain(10);
        let finalized = latest.clone();
        let (mut watcher, _rx) = l1_watcher(chain, vec![], vec![], finalized.clone(), latest);

        // When
        watcher.handle_finalized_block(&finalized).await?;

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_should_match_unfinalized_tail() -> eyre::Result<()> {
        // Given
        let (finalized, latest, chain) = chain(10);
        let (mut watcher, _) = l1_watcher(chain, vec![], vec![], finalized.clone(), latest.clone());

        // When
        watcher.handle_latest_block(&finalized, &latest).await?;

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 10);
        assert_eq!(watcher.unfinalized_blocks.pop().unwrap(), latest);

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_latest_block_should_extend_unfinalized_chain() -> eyre::Result<()> {
        // Given
        let (finalized, latest, chain) = chain(10);
        let unfinalized_chain = chain[..9].to_vec();
        let (mut watcher, _rx) =
            l1_watcher(unfinalized_chain, vec![], vec![], finalized.clone(), latest.clone());

        assert_eq!(watcher.unfinalized_blocks.len(), 9);

        // When
        watcher.handle_latest_block(&finalized, &latest).await?;

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 10);
        assert_eq!(watcher.unfinalized_blocks.pop().unwrap(), latest);

        Ok(())
    }

    #[tokio::test]
    async fn test_should_fetch_missing_unfinalized_blocks() -> eyre::Result<()> {
        // Given
        let (finalized, latest, chain) = chain(10);
        let unfinalized_chain = chain[..5].to_vec();
        let (mut watcher, mut receiver) =
            l1_watcher(unfinalized_chain, chain, vec![], finalized.clone(), latest.clone());

        // When
        watcher.handle_latest_block(&finalized, &latest).await?;

        // Then
        assert_eq!(watcher.unfinalized_blocks.len(), 10);
        assert_eq!(watcher.unfinalized_blocks.pop().unwrap(), latest);
        let notification = receiver.recv().await.unwrap();
        assert!(matches!(*notification, L1Notification::NewBlock(_)));

        Ok(())
    }

    #[tokio::test]
    async fn test_should_handle_latest_block_with_reorg() -> eyre::Result<()> {
        // Given
        let (finalized, _, chain) = chain(10);
        let reorged = chain_from(&chain[5], 10);
        let latest = reorged[9].clone();
        let (mut watcher, mut receiver) =
            l1_watcher(chain.clone(), reorged, vec![], finalized.clone(), latest.clone());

        // When
        watcher.current_block_number = chain[9].number;
        watcher.handle_latest_block(&finalized, &latest).await?;

        // Then
        assert_eq!(watcher.unfinalized_blocks.pop().unwrap(), latest);
        assert_eq!(watcher.current_block_number, chain[5].number);

        let notification = receiver.recv().await.unwrap();
        assert!(matches!(*notification, L1Notification::Reorg(_)));
        let notification = receiver.recv().await.unwrap();
        assert!(matches!(*notification, L1Notification::NewBlock(_)));

        Ok(())
    }

    #[tokio::test]
    async fn test_should_handle_l1_messages() -> eyre::Result<()> {
        // Given
        let (finalized, latest, chain) = chain(10);
        let (watcher, _) = l1_watcher(chain, vec![], vec![], finalized.clone(), latest.clone());

        // build test logs.
        let mut logs = (0..10).map(|_| random!(Log)).collect::<Vec<_>>();
        let mut queue_transaction = random!(Log);
        let mut inner_log = random!(alloy_primitives::Log);
        inner_log.data = random!(QueueTransaction).encode_log_data();
        queue_transaction.inner = inner_log;
        queue_transaction.block_number = Some(random!(u64));
        queue_transaction.block_timestamp = Some(random!(u64));
        logs.push(queue_transaction);

        // When
        let notification = watcher.handle_l1_messages(&logs).await?.pop().unwrap();

        // Then
        assert!(matches!(notification, L1Notification::L1Message { .. }));

        Ok(())
    }

    #[tokio::test]
    async fn test_should_handle_batch_commits() -> eyre::Result<()> {
        // Given
        let (finalized, latest, chain) = chain(10);

        // prepare the commit batch call transaction.
        let mut inner = random!(Signed<TxEip1559>);
        inner.tx_mut().input = random!(commitBatchCall).abi_encode().into();
        let recovered = Recovered::new_unchecked(inner.into(), random!(Address));
        let tx = Transaction {
            inner: recovered,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            effective_gas_price: None,
        };

        let (watcher, _) =
            l1_watcher(chain, vec![], vec![tx.clone()], finalized.clone(), latest.clone());

        // build test logs.
        let mut logs = (0..10).map(|_| random!(Log)).collect::<Vec<_>>();
        let mut batch_commit = random!(Log);
        let mut inner_log = random!(alloy_primitives::Log);
        inner_log.data =
            CommitBatch { batch_index: U256::from(random!(u64)), batch_hash: random!(B256) }
                .encode_log_data();
        batch_commit.inner = inner_log;
        batch_commit.transaction_hash = Some(*tx.inner.tx_hash());
        batch_commit.block_number = Some(random!(u64));
        batch_commit.block_timestamp = Some(random!(u64));
        logs.push(batch_commit);

        // When
        let notification = watcher.handle_batch_commits(&logs).await?.pop().unwrap();

        // Then
        assert!(matches!(notification, L1Notification::BatchCommit { .. }));

        Ok(())
    }

    #[tokio::test]
    async fn test_should_handle_finalize_commits() -> eyre::Result<()> {
        // Given
        let (finalized, latest, chain) = chain(10);
        let (watcher, _) = l1_watcher(chain, vec![], vec![], finalized.clone(), latest.clone());

        // build test logs.
        let mut logs = (0..10).map(|_| random!(Log)).collect::<Vec<_>>();
        let mut finalize_commit = random!(Log);
        let mut inner_log = random!(alloy_primitives::Log);
        let mut batch = random!(FinalizeBatch);
        batch.batch_index = U256::from(random!(u64));
        inner_log.data = batch.encode_log_data();
        finalize_commit.inner = inner_log;
        finalize_commit.block_number = Some(random!(u64));
        logs.push(finalize_commit);

        // When
        let notification = watcher.handle_batch_finalization(&logs).await?.pop().unwrap();

        // Then
        assert!(matches!(notification, L1Notification::BatchFinalization { .. }));

        Ok(())
    }
}
