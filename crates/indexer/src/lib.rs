//! A library responsible for indexing data relevant to the L1.

use alloy_consensus::Header;
use alloy_eips::BlockHashOrNumber;
use alloy_primitives::{b256, keccak256, B256};
use alloy_provider::Provider;
use futures::{task::AtomicWaker, Stream, StreamExt, TryStreamExt};
use reth_chainspec::EthChainSpec;
use reth_network_p2p::{BlockClient, BodiesClient};
use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::{
    BatchCommitData, BatchInfo, BlockInfo, BoundedVec, ChainImport, L1MessageEnvelope,
    L2BlockInfoWithL1Messages,
};
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_hardforks::{ScrollHardfork, ScrollHardforks};
use scroll_alloy_network::Scroll;
use scroll_db::{Database, DatabaseError, DatabaseOperations, UnwindResult};
use scroll_network::NewBlockWithPeer;
use std::{
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::Instant,
};
use strum::IntoEnumIterator;

mod action;
use action::{IndexerFuture, PendingIndexerFuture};

mod event;
pub use event::ChainOrchestratorEvent;

mod error;
pub use error::IndexerError;

mod metrics;
pub use metrics::{IndexerItem, IndexerMetrics};

const L1_MESSAGE_QUEUE_HASH_MASK: B256 =
    b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000");

/// The number of block headers we keep in memory.
const CHAIN_BUFFER_SIZE: usize = 2000;

/// The threshold for optimistic syncing. If the received block is more than this many blocks
/// ahead of the current chain, we optimistically sync the chain.
const OPTIMISTIC_SYNC_THRESHOLD: u64 = 100;

type Chain = BoundedVec<Header>;

/// The indexer is responsible for indexing data relevant to the L1.
#[derive(Debug)]
pub struct ChainOrchestrator<ChainSpec, BC, P> {
    /// The `BlockClient` that is used to fetch blocks from peers over p2p.
    network_client: Arc<BC>,
    /// The L2 client that is used to interact with the L2 chain.
    l2_client: Arc<P>,
    /// An in-memory representation of the optimistic chain we are following.
    chain: Arc<Mutex<Chain>>,
    /// A reference to the database used to persist the indexed data.
    database: Arc<Database>,
    /// A queue of pending futures.
    pending_futures: VecDeque<IndexerFuture>,
    /// The block number of the L1 finalized block.
    l1_finalized_block_number: Arc<AtomicU64>,
    /// The block number of the L2 finalized block.
    l2_finalized_block_number: Arc<AtomicU64>,
    /// The chain specification for the indexer.
    chain_spec: Arc<ChainSpec>,
    /// The metrics for the indexer.
    metrics: HashMap<IndexerItem, IndexerMetrics>,
    /// A boolean to represent if the [`ChainOrchestrator`] is in optimistic mode.
    optimistic_mode: Arc<Mutex<bool>>,
    /// A boolean to represent if the L1 has been synced.
    l1_synced: bool,
    /// The waker to notify when the engine driver should be polled.
    waker: AtomicWaker,
}

impl<
        ChainSpec: ScrollHardforks + EthChainSpec + Send + Sync + 'static,
        BC: BlockClient<Block = ScrollBlock> + Send + Sync + 'static,
        P: Provider<Scroll> + 'static,
    > ChainOrchestrator<ChainSpec, BC, P>
{
    /// Creates a new indexer with the given [`Database`].
    pub async fn new(
        database: Arc<Database>,
        chain_spec: Arc<ChainSpec>,
        block_client: BC,
        l2_client: P,
    ) -> Self {
        let chain = init_chain_from_db(&database, &l2_client).await;
        Self {
            network_client: Arc::new(block_client),
            l2_client: Arc::new(l2_client),
            chain: Arc::new(Mutex::new(chain)),
            database,
            pending_futures: Default::default(),
            l1_finalized_block_number: Arc::new(AtomicU64::new(0)),
            l2_finalized_block_number: Arc::new(AtomicU64::new(0)),
            chain_spec,
            metrics: IndexerItem::iter()
                .map(|i| {
                    let label = i.as_str();
                    (i, IndexerMetrics::new_with_labels(&[("item", label)]))
                })
                .collect(),
            optimistic_mode: Arc::new(Mutex::new(false)),
            l1_synced: false,
            waker: AtomicWaker::new(),
        }
    }

    /// Wraps a pending indexer future, metering the completion of it.
    pub fn handle_metered(
        &mut self,
        item: IndexerItem,
        indexer_fut: PendingIndexerFuture,
    ) -> PendingIndexerFuture {
        let metric = self.metrics.get(&item).expect("metric exists").clone();
        let fut_wrapper = Box::pin(async move {
            let now = Instant::now();
            let res = indexer_fut.await;
            metric.task_duration.record(now.elapsed().as_secs_f64());
            res
        });
        fut_wrapper
    }

    /// Sets the L1 synced status to the provided value.
    pub fn set_l1_synced_status(&mut self, l1_synced: bool) {
        self.l1_synced = l1_synced;
    }

    /// Handles a new block received from a peer.
    pub fn handle_block_from_peer(&mut self, block_with_peer: NewBlockWithPeer) {
        let chain = self.chain.clone();
        let l2_client = self.l2_client.clone();
        let optimistic_mode = self.optimistic_mode.clone();
        let network_client = self.network_client.clone();
        let database = self.database.clone();
        let fut = self.handle_metered(
            IndexerItem::NewBlock,
            Box::pin(async move {
                Self::handle_new_block(
                    chain,
                    l2_client,
                    optimistic_mode,
                    network_client,
                    database,
                    block_with_peer,
                )
                .await
            }),
        );
        self.pending_futures.push_back(IndexerFuture::HandleL2Block(fut));
        self.waker.wake();
    }

    /// Handles a new block received from the network.
    pub async fn handle_new_block(
        chain: Arc<Mutex<Chain>>,
        l2_client: Arc<P>,
        optimistic_mode: Arc<Mutex<bool>>,
        network_client: Arc<BC>,
        database: Arc<Database>,
        block_with_peer: NewBlockWithPeer,
    ) -> Result<ChainOrchestratorEvent, IndexerError> {
        let NewBlockWithPeer { block: received_block, peer_id, signature } = block_with_peer;
        let mut current_chain_headers = std::mem::take(&mut *chain.lock().unwrap());
        let max_block_number = current_chain_headers.last().expect("chain can not be empty").number;
        let min_block_number =
            current_chain_headers.first().expect("chain can not be empty").number;
        let optimistic_mode_local: bool = {
            let guard = optimistic_mode.lock().unwrap();
            *guard
        };

        // If the received block has a block number that is greater than the tip
        // of the chain by the optimistic sync threshold, we optimistically sync the chain and
        // update the in-memory buffer.
        if (received_block.header.number - max_block_number) >= OPTIMISTIC_SYNC_THRESHOLD {
            // fetch the latest `OPTIMISTIC_CHAIN_BUFFER_SIZE` blocks from the network for the
            // optimistic chain.
            let mut optimistic_headers = vec![received_block.header.clone()];
            while optimistic_headers.len() < CHAIN_BUFFER_SIZE &&
                optimistic_headers.last().unwrap().number != 0
            {
                tracing::trace!(target: "scroll::watcher", number = ?(optimistic_headers.last().unwrap().number - 1), "fetching block");
                let header = network_client
                    .get_header(BlockHashOrNumber::Hash(
                        optimistic_headers.last().unwrap().parent_hash,
                    ))
                    .await
                    .unwrap()
                    .into_data()
                    .unwrap();
                optimistic_headers.push(header);
            }
            optimistic_headers.reverse();
            let mut new_chain = Chain::new(CHAIN_BUFFER_SIZE);
            new_chain.extend(optimistic_headers);
            *chain.lock().unwrap() = new_chain;
            *optimistic_mode.lock().unwrap() = true;
            return Ok(ChainOrchestratorEvent::OptimisticSync(received_block));
        }

        // Check if we have already have this block in memory.
        if received_block.number <= max_block_number &&
            received_block.number >= min_block_number &&
            current_chain_headers.iter().any(|h| h == &received_block.header)
        {
            tracing::debug!(target: "scroll::watcher", block_hash = ?received_block.header.hash_slow(), "block already in chain");
            return Ok(ChainOrchestratorEvent::BlockAlreadyKnown(
                received_block.header.hash_slow(),
                peer_id,
            ));
        }

        // Perform preliminary checks on received block that are dependent on database state due to
        // not being in in-memory chain.
        if received_block.header.number <= min_block_number {
            // If we are in optimistic mode, we return an event indicating that we have insufficient
            // data to process the block.
            if optimistic_mode_local {
                return Ok(ChainOrchestratorEvent::InsufficientDataForReceivedBlock(
                    received_block.header.hash_slow(),
                ));
            }

            // We check the database to see if this block is already known.
            if database
                .get_l2_block_and_batch_info_by_hash(received_block.header.hash_slow())
                .await?
                .is_some()
            {
                tracing::debug!(target: "scroll::watcher", block_hash = ?received_block.header.hash_slow(), "block already in database");
                return Ok(ChainOrchestratorEvent::BlockAlreadyKnown(
                    received_block.header.hash_slow(),
                    peer_id,
                ));
            }
        };

        let mut new_chain_headers = vec![received_block.header.clone()];
        let mut new_header_tail = received_block.header.clone();

        enum ReorgIndex {
            Memory(usize),
            Database(BlockInfo),
        }

        // We should never have a re-org that is deeper than the current safe head.
        let (latest_safe_block, _) =
            database.get_latest_safe_l2_info().await?.expect("safe block must exist");

        // We search for the re-org index in the in-memory chain or the database.
        let reorg_index = {
            loop {
                // If the current header block number is greater than the in-memory chain then we
                // should search the in-memory chain.
                if new_header_tail.number >= min_block_number {
                    if let Some(pos) = current_chain_headers
                        .iter()
                        .rposition(|h| h.hash_slow() == new_header_tail.parent_hash)
                    {
                        // If the received fork is older than the current chain, we return an event
                        // indicating that we have received an old fork.
                        if (pos < current_chain_headers.len() - 1) &&
                            current_chain_headers.get(pos + 1).unwrap().timestamp >=
                                new_header_tail.timestamp
                        {
                            return Ok(ChainOrchestratorEvent::OldForkReceived {
                                headers: new_chain_headers,
                                peer_id,
                                signature,
                            });
                        }
                        break Some(ReorgIndex::Memory(pos));
                    }
                // If we are in optimistic mode, we terminate the search as we don't have the
                // necessary data in the database. This is fine because very deep
                // re-orgs are rare and in any case will be resolved once optimistic sync is
                // completed.If the current header block number is less than the
                // latest safe block number then this would suggest a reorg of a
                // safe block which is not invalid - terminate the search.
                } else if optimistic_mode_local ||
                    new_header_tail.number <= latest_safe_block.number
                {
                    break None

                // If the block is not in the in-memory chain, we search the database.
                } else if let Some((l2_block, _batch_info)) = database
                    .get_l2_block_and_batch_info_by_hash(new_header_tail.parent_hash)
                    .await?
                {
                    let diverged_block = database
                        .get_l2_block_info_by_number(l2_block.number + 1)
                        .await?
                        .expect("diverged block must exist");
                    let diverged_block =
                        l2_client.get_block_by_hash(diverged_block.hash).await.unwrap().unwrap();

                    // If the received fork is older than the current chain, we return an event
                    // indicating that we have received an old fork.
                    if diverged_block.header.timestamp >= new_header_tail.timestamp {
                        return Ok(ChainOrchestratorEvent::OldForkReceived {
                            headers: new_chain_headers,
                            peer_id,
                            signature,
                        });
                    }

                    break Some(ReorgIndex::Database(l2_block))
                }

                tracing::trace!(target: "scroll::watcher", number = ?(new_header_tail.number - 1), "fetching block");
                let header = network_client
                    .get_header(BlockHashOrNumber::Hash(new_header_tail.parent_hash))
                    .await
                    .unwrap()
                    .into_data()
                    .unwrap();
                new_chain_headers.push(header.clone());
                new_header_tail = header;
            }
        };

        // Reverse the new chain headers to have them in the correct order.
        new_chain_headers.reverse();

        // Fetch the blocks associated with the new chain headers.
        let new_blocks = if new_chain_headers.len() == 1 {
            vec![received_block]
        } else {
            fetch_blocks(new_chain_headers.clone(), network_client.clone()).await
        };

        // If we are not in optimistic mode, we validate the L1 messages in the new blocks.
        if !optimistic_mode_local {
            validate_l1_messages(&new_blocks, database.clone()).await?;
        }

        match reorg_index {
            // If this is a simple chain extension, we can just extend the in-memory chain and emit
            // a ChainExtended event.
            Some(ReorgIndex::Memory(index)) if index == current_chain_headers.len() - 1 => {
                // Update the chain with the new blocks.
                current_chain_headers.extend(new_blocks.iter().map(|b| b.header.clone()));
                *chain.lock().unwrap() = current_chain_headers;

                Ok(ChainOrchestratorEvent::ChainExtended(ChainImport::new(
                    new_blocks, peer_id, signature,
                )))
            }
            // If we are re-organizing the in-memory chain, we need to split the chain at the reorg
            // point and extend it with the new blocks.
            Some(ReorgIndex::Memory(position)) => {
                // reorg the in-memory chain to the new chain and issue a reorg event.
                let mut new_chain = Chain::new(CHAIN_BUFFER_SIZE);
                new_chain.extend(current_chain_headers.iter().take(position).cloned());
                new_chain.extend(new_chain_headers);
                *chain.lock().unwrap() = new_chain;

                Ok(ChainOrchestratorEvent::ChainReorged(ChainImport::new(
                    new_blocks, peer_id, signature,
                )))
            }
            // If we have a deep reorg that impacts the database, we need to delete the blocks from
            // the database, update the in-memory chain and emit a ChainReorged event.
            Some(ReorgIndex::Database(l2_block)) => {
                // remove old chain data from the database.
                database.delete_l2_blocks_gt(l2_block.number).await?;

                // Update the in-memory chain with the new blocks.
                let mut new_chain = Chain::new(CHAIN_BUFFER_SIZE);
                new_chain.extend(new_chain_headers);
                *chain.lock().unwrap() = new_chain;

                Ok(ChainOrchestratorEvent::ChainReorged(ChainImport::new(
                    new_blocks, peer_id, signature,
                )))
            }
            None => Err(IndexerError::L2SafeBlockReorgDetected),
        }
    }

    /// Inserts an L2 block in the database.
    pub fn consolidate_l2_blocks(
        &mut self,
        block_info: Vec<L2BlockInfoWithL1Messages>,
        batch_info: Option<BatchInfo>,
    ) {
        let database = self.database.clone();
        let l1_synced = self.l1_synced;
        let optimistic_mode = self.optimistic_mode.clone();
        let chain = self.chain.clone();
        let fut = self.handle_metered(
            IndexerItem::InsertL2Block,
            Box::pin(async move {
                // If we are in optimistic mode and the L1 is synced, we consolidate the chain and
                // disable optimistic mode.
                if l1_synced && *optimistic_mode.lock().unwrap() {
                    consolidate_chain(database.clone(), chain).await?;
                }
                let head = block_info.last().expect("block info must not be empty").clone();
                database.insert_blocks(block_info, batch_info).await?;
                *optimistic_mode.lock().unwrap() = false;
                Result::<_, IndexerError>::Ok(ChainOrchestratorEvent::L2BlockCommitted(
                    head, batch_info,
                ))
            }),
        );

        self.pending_futures.push_back(IndexerFuture::HandleDerivedBlock(fut))
    }

    /// Handles an event from the L1.
    pub fn handle_l1_notification(&mut self, event: L1Notification) {
        let fut = match event {
            L1Notification::Reorg(block_number) => IndexerFuture::HandleReorg(self.handle_metered(
                IndexerItem::L1Reorg,
                Box::pin(Self::handle_l1_reorg(
                    self.database.clone(),
                    self.chain_spec.clone(),
                    block_number,
                )),
            )),
            L1Notification::NewBlock(_) | L1Notification::Consensus(_) => return,
            L1Notification::Finalized(block_number) => {
                IndexerFuture::HandleFinalized(self.handle_metered(
                    IndexerItem::L1Finalization,
                    Box::pin(Self::handle_finalized(
                        self.database.clone(),
                        block_number,
                        self.l1_finalized_block_number.clone(),
                        self.l2_finalized_block_number.clone(),
                    )),
                ))
            }
            L1Notification::BatchCommit(batch) => {
                IndexerFuture::HandleBatchCommit(self.handle_metered(
                    IndexerItem::BatchCommit,
                    Box::pin(Self::handle_batch_commit(self.database.clone(), batch)),
                ))
            }
            L1Notification::L1Message { message, block_number, block_timestamp } => {
                IndexerFuture::HandleL1Message(self.handle_metered(
                    IndexerItem::L1Message,
                    Box::pin(Self::handle_l1_message(
                        self.database.clone(),
                        self.chain_spec.clone(),
                        message,
                        block_number,
                        block_timestamp,
                    )),
                ))
            }
            L1Notification::BatchFinalization { hash, block_number, .. } => {
                IndexerFuture::HandleBatchFinalization(self.handle_metered(
                    IndexerItem::BatchFinalization,
                    Box::pin(Self::handle_batch_finalization(
                        self.database.clone(),
                        hash,
                        block_number,
                        self.l1_finalized_block_number.clone(),
                        self.l2_finalized_block_number.clone(),
                    )),
                ))
            }
        };

        self.pending_futures.push_back(fut);
    }

    /// Handles a reorganization event by deleting all indexed data which is greater than the
    /// provided block number.
    async fn handle_l1_reorg(
        database: Arc<Database>,
        chain_spec: Arc<ChainSpec>,
        l1_block_number: u64,
    ) -> Result<ChainOrchestratorEvent, IndexerError> {
        let txn = database.tx().await?;
        let UnwindResult { l1_block_number, queue_index, l2_head_block_info, l2_safe_block_info } =
            txn.unwind(chain_spec.genesis_hash(), l1_block_number).await?;
        txn.commit().await?;
        Ok(ChainOrchestratorEvent::ChainUnwound {
            l1_block_number,
            queue_index,
            l2_head_block_info,
            l2_safe_block_info,
        })
    }

    /// Handles a finalized event by updating the indexer L1 finalized block and returning the new
    /// finalized L2 chain block.
    async fn handle_finalized(
        database: Arc<Database>,
        block_number: u64,
        l1_block_number: Arc<AtomicU64>,
        l2_block_number: Arc<AtomicU64>,
    ) -> Result<ChainOrchestratorEvent, IndexerError> {
        // Set the latest finalized L1 block in the database.
        database.set_latest_finalized_l1_block_number(block_number).await?;

        // get the newest finalized batch.
        let batch_hash = database.get_finalized_batch_hash_at_height(block_number).await?;

        // get the finalized block for the batch.
        let finalized_block = if let Some(hash) = batch_hash {
            Self::fetch_highest_finalized_block(database, hash, l2_block_number).await?
        } else {
            None
        };

        // update the indexer l1 block number.
        l1_block_number.store(block_number, Ordering::Relaxed);

        Ok(ChainOrchestratorEvent::L1BlockFinalized(block_number, finalized_block))
    }

    /// Handles an L1 message by inserting it into the database.
    async fn handle_l1_message(
        database: Arc<Database>,
        chain_spec: Arc<ChainSpec>,
        l1_message: TxL1Message,
        l1_block_number: u64,
        block_timestamp: u64,
    ) -> Result<ChainOrchestratorEvent, IndexerError> {
        let event = ChainOrchestratorEvent::L1MessageCommitted(l1_message.queue_index);

        let queue_hash = if chain_spec
            .scroll_fork_activation(ScrollHardfork::EuclidV2)
            .active_at_timestamp_or_number(block_timestamp, l1_block_number) &&
            l1_message.queue_index > 0
        {
            let index = l1_message.queue_index - 1;
            let prev_queue_hash = database
                .get_l1_message_by_index(index)
                .await?
                .map(|m| m.queue_hash)
                .ok_or(DatabaseError::L1MessageNotFound(index))?;

            let mut input = prev_queue_hash.unwrap_or_default().to_vec();
            input.append(&mut l1_message.tx_hash().to_vec());
            Some(keccak256(input) & L1_MESSAGE_QUEUE_HASH_MASK)
        } else {
            None
        };

        let l1_message = L1MessageEnvelope::new(l1_message, l1_block_number, None, queue_hash);
        database.insert_l1_message(l1_message).await?;
        Ok(event)
    }

    /// Handles a batch input by inserting it into the database.
    async fn handle_batch_commit(
        database: Arc<Database>,
        batch: BatchCommitData,
    ) -> Result<ChainOrchestratorEvent, IndexerError> {
        let event = ChainOrchestratorEvent::BatchCommitted(BatchInfo::new(batch.index, batch.hash));
        database.insert_batch(batch).await?;
        Ok(event)
    }

    /// Handles a batch finalization event by updating the batch input in the database.
    async fn handle_batch_finalization(
        database: Arc<Database>,
        batch_hash: B256,
        block_number: u64,
        l1_block_number: Arc<AtomicU64>,
        l2_block_number: Arc<AtomicU64>,
    ) -> Result<ChainOrchestratorEvent, IndexerError> {
        // finalized the batch.
        database.finalize_batch(batch_hash, block_number).await?;

        // check if the block where the batch was finalized is finalized on L1.
        let mut finalized_block = None;
        let l1_block_number_value = l1_block_number.load(Ordering::Relaxed);
        if l1_block_number_value > block_number {
            // fetch the finalized block.
            finalized_block =
                Self::fetch_highest_finalized_block(database, batch_hash, l2_block_number).await?;
        }

        let event = ChainOrchestratorEvent::BatchFinalized(batch_hash, finalized_block);
        Ok(event)
    }

    /// Returns the highest finalized block for the provided batch hash. Will return [`None`] if the
    /// block number has already been seen by the indexer.
    async fn fetch_highest_finalized_block(
        database: Arc<Database>,
        batch_hash: B256,
        l2_block_number: Arc<AtomicU64>,
    ) -> Result<Option<BlockInfo>, IndexerError> {
        let finalized_block = database.get_highest_block_for_batch(batch_hash).await?;

        // only return the block if the indexer hasn't seen it.
        // in which case also update the `l2_finalized_block_number` value.
        Ok(finalized_block.filter(|info| {
            let current_l2_block_number = l2_block_number.load(Ordering::Relaxed);
            if info.number > current_l2_block_number {
                l2_block_number.store(info.number, Ordering::Relaxed);
                true
            } else {
                false
            }
        }))
    }
}

async fn init_chain_from_db<P: Provider<Scroll> + 'static>(
    database: &Arc<Database>,
    l2_client: &P,
) -> BoundedVec<Header> {
    let blocks = {
        let mut blocks = Vec::with_capacity(CHAIN_BUFFER_SIZE);
        let mut blocks_stream = database.get_l2_blocks().await.unwrap().take(CHAIN_BUFFER_SIZE);
        while let Some(block_info) = blocks_stream.try_next().await.unwrap() {
            let header = l2_client
                .get_block_by_hash(block_info.hash)
                .await
                .unwrap()
                .unwrap()
                .header
                .into_consensus();
            blocks.push(header);
        }
        blocks.reverse();
        blocks
    };
    let mut chain: Chain = Chain::new(CHAIN_BUFFER_SIZE);
    chain.extend(blocks);
    chain
}

/// Unwinds the indexer by deleting all indexed data greater than the provided L1 block number.
pub async fn unwind<ChainSpec: ScrollHardforks + EthChainSpec + Send + Sync + 'static>(
    database: Arc<Database>,
    chain_spec: Arc<ChainSpec>,
    l1_block_number: u64,
) -> Result<ChainOrchestratorEvent, IndexerError> {
    // create a database transaction so this operation is atomic
    let txn = database.tx().await?;

    // delete batch inputs and l1 messages
    let batches_removed = txn.delete_batches_gt(l1_block_number).await?;
    let deleted_messages = txn.delete_l1_messages_gt(l1_block_number).await?;

    // filter and sort the executed L1 messages
    let mut removed_executed_l1_messages: Vec<_> =
        deleted_messages.into_iter().filter(|x| x.l2_block_number.is_some()).collect();
    removed_executed_l1_messages
        .sort_by(|a, b| a.transaction.queue_index.cmp(&b.transaction.queue_index));

    // check if we need to reorg the L2 head and delete some L2 blocks
    let (queue_index, l2_head_block_info) = if let Some(msg) = removed_executed_l1_messages.first()
    {
        let l2_reorg_block_number = msg
            .l2_block_number
            .expect("we guarantee that this is Some(u64) due to the filter on line 130") -
            1;
        let l2_block_info = txn
            .get_l2_block_info_by_number(l2_reorg_block_number)
            .await?
            .ok_or(IndexerError::L2BlockNotFound(l2_reorg_block_number))?;
        txn.delete_l2_blocks_gt(l2_reorg_block_number).await?;
        (Some(msg.transaction.queue_index), Some(l2_block_info))
    } else {
        (None, None)
    };

    // check if we need to reorg the L2 safe block
    let l2_safe_block_info = if batches_removed > 0 {
        if let Some(x) = txn.get_latest_safe_l2_info().await? {
            Some(x.0)
        } else {
            Some(BlockInfo::new(0, chain_spec.genesis_hash()))
        }
    } else {
        None
    };

    // commit the transaction
    txn.commit().await?;
    Ok(ChainOrchestratorEvent::ChainUnwound {
        l1_block_number,
        queue_index,
        l2_head_block_info,
        l2_safe_block_info,
    })
}

impl<
        ChainSpec: ScrollHardforks + 'static,
        BC: BlockClient<Block = ScrollBlock> + Send + Sync + 'static,
        P: Provider<Scroll> + Send + Sync + 'static,
    > Stream for ChainOrchestrator<ChainSpec, BC, P>
{
    type Item = Result<ChainOrchestratorEvent, IndexerError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Register the waker such that we can wake when required.
        self.waker.register(cx.waker());

        // Remove and poll the next future in the queue
        if let Some(mut action) = self.pending_futures.pop_front() {
            return match action.poll(cx) {
                Poll::Ready(result) => Poll::Ready(Some(result)),
                Poll::Pending => {
                    self.pending_futures.push_front(action);
                    Poll::Pending
                }
            };
        }

        Poll::Pending
    }
}

async fn consolidate_chain(
    _database: Arc<Database>,
    _chain: Arc<Mutex<Chain>>,
) -> Result<(), IndexerError> {
    // TODO: implement the logic to consolidate the chain.
    // If we are in optimistic mode, we consolidate the chain and disable optimistic
    // mode.
    // let mut chain = chain.lock().unwrap();
    // if !chain.is_empty() {
    //     database.insert_chain(chain.drain(..).collect()).await?;
    // }
    Ok(())
}

async fn fetch_blocks<BC: BlockClient<Block = ScrollBlock> + Send + Sync + 'static>(
    headers: Vec<Header>,
    client: Arc<BC>,
) -> Vec<ScrollBlock> {
    let mut blocks = Vec::new();
    // TODO: migrate to `get_block_bodies_with_range_hint`.
    let bodies = client
        .get_block_bodies(headers.iter().map(|h| h.hash_slow()).collect())
        .await
        .expect("Failed to fetch block bodies")
        .into_data();

    // TODO: can we assume the bodies are in the same order as the headers?
    for (header, body) in headers.into_iter().zip(bodies) {
        blocks.push(ScrollBlock::new(header, body));
    }

    blocks
}

async fn validate_l1_messages(
    _blocks: &[ScrollBlock],
    _database: Arc<Database>,
) -> Result<(), IndexerError> {
    // TODO: implement L1 message validation logic.
    Ok(())
}

#[cfg(test)]
mod test {
    use std::vec;

    use super::*;
    use alloy_consensus::Header;
    use alloy_eips::{BlockHashOrNumber, BlockNumHash};
    use alloy_primitives::{address, bytes, B256, U256};
    use alloy_provider::{ProviderBuilder, RootProvider};
    use alloy_transport::mock::Asserter;
    use arbitrary::{Arbitrary, Unstructured};
    use futures::StreamExt;
    use parking_lot::Mutex;
    use rand::Rng;
    use reth_eth_wire_types::HeadersDirection;
    use reth_network_p2p::{
        download::DownloadClient,
        error::PeerRequestResult,
        headers::client::{HeadersClient, HeadersRequest},
        priority::Priority,
        BodiesClient,
    };
    use reth_network_peers::{PeerId, WithPeerId};
    use reth_primitives_traits::Block;
    use reth_scroll_chainspec::{ScrollChainSpec, SCROLL_MAINNET};
    use rollup_node_primitives::BatchCommitData;
    use scroll_alloy_network::Scroll;
    use scroll_db::test_utils::setup_test_db;
    use std::{collections::HashMap, ops::RangeInclusive, sync::Arc};

    type ScrollBody = <ScrollBlock as Block>::Body;

    /// A headers+bodies client that stores the headers and bodies in memory, with an artificial
    /// soft bodies response limit that is set to 20 by default.
    ///
    /// This full block client can be [Clone]d and shared between multiple tasks.
    #[derive(Clone, Debug)]
    struct TestScrollFullBlockClient {
        headers: Arc<Mutex<HashMap<B256, Header>>>,
        bodies: Arc<Mutex<HashMap<B256, <ScrollBlock as Block>::Body>>>,
        // soft response limit, max number of bodies to respond with
        soft_limit: usize,
    }

    impl Default for TestScrollFullBlockClient {
        fn default() -> Self {
            let mainnet_genesis: reth_scroll_primitives::ScrollBlock =
                serde_json::from_str(include_str!("../testdata/genesis_block.json")).unwrap();
            let (header, body) = mainnet_genesis.split();
            let hash = header.hash_slow();
            let headers = HashMap::from([(hash, header)]);
            let bodies = HashMap::from([(hash, body)]);
            Self {
                headers: Arc::new(Mutex::new(headers)),
                bodies: Arc::new(Mutex::new(bodies)),
                soft_limit: 20,
            }
        }
    }

    // impl TestScrollFullBlockClient {
    //     /// Insert a header and body into the client maps.
    //     fn insert(&self, header: SealedHeader, body: ScrollBody) {
    //         let hash = header.hash();
    //         self.headers.lock().insert(hash, header.unseal());
    //         self.bodies.lock().insert(hash, body);
    //     }

    //     /// Set the soft response limit.
    //     const fn set_soft_limit(&mut self, limit: usize) {
    //         self.soft_limit = limit;
    //     }

    //     /// Get the block with the highest block number.
    //     fn highest_block(&self) -> Option<SealedBlock<ScrollBlock>> {
    //         self.headers.lock().iter().max_by_key(|(_, header)| header.number).and_then(
    //             |(hash, header)| {
    //                 self.bodies.lock().get(hash).map(|body| {
    //                     SealedBlock::from_parts_unchecked(header.clone(), body.clone(), *hash)
    //                 })
    //             },
    //         )
    //     }
    // }

    impl DownloadClient for TestScrollFullBlockClient {
        /// Reports a bad message from a specific peer.
        fn report_bad_message(&self, _peer_id: PeerId) {}

        /// Retrieves the number of connected peers.
        ///
        /// Returns the number of connected peers in the test scenario (1).
        fn num_connected_peers(&self) -> usize {
            1
        }
    }

    /// Implements the `HeadersClient` trait for the `TestFullBlockClient` struct.
    impl HeadersClient for TestScrollFullBlockClient {
        type Header = Header;
        /// Specifies the associated output type.
        type Output = futures::future::Ready<PeerRequestResult<Vec<Header>>>;

        /// Retrieves headers with a given priority level.
        ///
        /// # Arguments
        ///
        /// * `request` - A `HeadersRequest` indicating the headers to retrieve.
        /// * `_priority` - A `Priority` level for the request.
        ///
        /// # Returns
        ///
        /// A `Ready` future containing a `PeerRequestResult` with a vector of retrieved headers.
        fn get_headers_with_priority(
            &self,
            request: HeadersRequest,
            _priority: Priority,
        ) -> Self::Output {
            let headers = self.headers.lock();

            // Initializes the block hash or number.
            let mut block: BlockHashOrNumber = match request.start {
                BlockHashOrNumber::Hash(hash) => headers.get(&hash).cloned(),
                BlockHashOrNumber::Number(num) => {
                    headers.values().find(|h| h.number == num).cloned()
                }
            }
            .map(|h| h.number.into())
            .unwrap();

            // Retrieves headers based on the provided limit and request direction.
            let resp = (0..request.limit)
                .filter_map(|_| {
                    headers.iter().find_map(|(hash, header)| {
                        // Checks if the header matches the specified block or number.
                        BlockNumHash::new(header.number, *hash).matches_block_or_num(&block).then(
                            || {
                                match request.direction {
                                    HeadersDirection::Falling => block = header.parent_hash.into(),
                                    HeadersDirection::Rising => block = (header.number + 1).into(),
                                }
                                header.clone()
                            },
                        )
                    })
                })
                .collect::<Vec<_>>();

            // Returns a future containing the retrieved headers with a random peer ID.
            futures::future::ready(Ok(WithPeerId::new(PeerId::random(), resp)))
        }
    }

    /// Implements the `BodiesClient` trait for the `TestFullBlockClient` struct.
    impl BodiesClient for TestScrollFullBlockClient {
        type Body = ScrollBody;
        /// Defines the output type of the function.
        type Output = futures::future::Ready<PeerRequestResult<Vec<Self::Body>>>;

        /// Retrieves block bodies corresponding to provided hashes with a given priority.
        ///
        /// # Arguments
        ///
        /// * `hashes` - A vector of block hashes to retrieve bodies for.
        /// * `_priority` - Priority level for block body retrieval (unused in this implementation).
        ///
        /// # Returns
        ///
        /// A future containing the result of the block body retrieval operation.
        fn get_block_bodies_with_priority_and_range_hint(
            &self,
            hashes: Vec<B256>,
            _priority: Priority,
            _range_hint: Option<RangeInclusive<u64>>,
        ) -> Self::Output {
            // Acquire a lock on the bodies.
            let bodies = self.bodies.lock();

            // Create a future that immediately returns the result of the block body retrieval
            // operation.
            futures::future::ready(Ok(WithPeerId::new(
                PeerId::random(),
                hashes
                    .iter()
                    .filter_map(|hash| bodies.get(hash).cloned())
                    .take(self.soft_limit)
                    .collect(),
            )))
        }
    }

    impl BlockClient for TestScrollFullBlockClient {
        type Block = ScrollBlock;
    }

    async fn setup_test_indexer() -> (
        ChainOrchestrator<ScrollChainSpec, TestScrollFullBlockClient, RootProvider<Scroll>>,
        Arc<Database>,
    ) {
        // Get a provider to the node.
        // TODO: update to use a real node URL.
        let assertor = Asserter::new();
        let mainnet_genesis: <Scroll as scroll_alloy_network::Network>::BlockResponse =
            serde_json::from_str(include_str!("../testdata/genesis_block_rpc.json"))
                .expect("Failed to parse mainnet genesis block");
        assertor.push_success(&mainnet_genesis);
        let provider = ProviderBuilder::<_, _, Scroll>::default().connect_mocked_client(assertor);
        let db = Arc::new(setup_test_db().await);
        (
            ChainOrchestrator::new(
                db.clone(),
                SCROLL_MAINNET.clone(),
                TestScrollFullBlockClient::default(),
                provider,
            )
            .await,
            db,
        )
    }

    #[tokio::test]
    async fn test_handle_commit_batch() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let batch_commit = BatchCommitData::arbitrary(&mut u).unwrap();
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit.clone()));

        let _ = indexer.next().await;

        let batch_commit_result = db.get_batch_by_index(batch_commit.index).await.unwrap().unwrap();

        assert_eq!(batch_commit, batch_commit_result);
    }

    #[tokio::test]
    async fn test_handle_l1_message() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let message = TxL1Message {
            queue_index: i64::arbitrary(&mut u).unwrap().unsigned_abs(),
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let block_number = u64::arbitrary(&mut u).unwrap();
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: message.clone(),
            block_number,
            block_timestamp: 0,
        });

        let _ = indexer.next().await;

        let l1_message_result =
            db.get_l1_message_by_index(message.queue_index).await.unwrap().unwrap();
        let l1_message = L1MessageEnvelope::new(message, block_number, None, None);

        assert_eq!(l1_message, l1_message_result);
    }

    #[tokio::test]
    async fn test_l1_message_hash_queue() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // insert the previous L1 message in database.
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: TxL1Message { queue_index: 1062109, ..Default::default() },
            block_number: 1475588,
            block_timestamp: 1745305199,
        });
        let _ = indexer.next().await.unwrap().unwrap();

        // <https://sepolia.scrollscan.com/tx/0xd80cd61ac5d8665919da19128cc8c16d3647e1e2e278b931769e986d01c6b910>
        let message = TxL1Message {
            queue_index: 1062110,
            gas_limit: 168000,
            to: address!("Ba50f5340FB9F3Bd074bD638c9BE13eCB36E603d"),
            value: U256::ZERO,
            sender: address!("61d8d3E7F7c656493d1d76aAA1a836CEdfCBc27b"),
            input: bytes!("8ef1332e000000000000000000000000323522a8de3cddeddbb67094eecaebc2436d6996000000000000000000000000323522a8de3cddeddbb67094eecaebc2436d699600000000000000000000000000000000000000000000000000038d7ea4c6800000000000000000000000000000000000000000000000000000000000001034de00000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000"),
        };
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: message.clone(),
            block_number: 14755883,
            block_timestamp: 1745305200,
        });

        let _ = indexer.next().await.unwrap().unwrap();

        let l1_message_result =
            db.get_l1_message_by_index(message.queue_index).await.unwrap().unwrap();

        assert_eq!(
            b256!("5e48ae1092c7f912849b9935f4e66870d2034b24fb2016f506e6754900000000"),
            l1_message_result.queue_hash.unwrap()
        );
    }

    #[tokio::test]
    async fn test_handle_reorg() {
        // Instantiate indexer and db
        let (mut indexer, db) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a 3 random batch inputs and set their block numbers
        let mut batch_commit_block_1 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_1.block_number = 1;
        let batch_commit_block_1 = batch_commit_block_1;

        let mut batch_commit_block_20 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_20.block_number = 20;
        let batch_commit_block_20 = batch_commit_block_20;

        let mut batch_commit_block_30 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_30.block_number = 30;
        let batch_commit_block_30 = batch_commit_block_30;

        // Index batch inputs
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_20.clone()));
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_30.clone()));

        // Generate 3 random L1 messages and set their block numbers
        let l1_message_block_1 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 1,
            l2_block_number: None,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let l1_message_block_20 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 20,
            l2_block_number: None,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let l1_message_block_30 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 30,
            l2_block_number: None,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };

        // Index L1 messages
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_1.clone().transaction,
            block_number: l1_message_block_1.clone().l1_block_number,
            block_timestamp: 0,
        });
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_20.clone().transaction,
            block_number: l1_message_block_20.clone().l1_block_number,
            block_timestamp: 0,
        });
        indexer.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_30.clone().transaction,
            block_number: l1_message_block_30.clone().l1_block_number,
            block_timestamp: 0,
        });

        // Reorg at block 20
        indexer.handle_l1_notification(L1Notification::Reorg(20));

        for _ in 0..7 {
            indexer.next().await.unwrap().unwrap();
        }

        // Check that the batch input at block 30 is deleted
        let batch_commits =
            db.get_batches().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;

        assert_eq!(3, batch_commits.len());
        assert!(batch_commits.contains(&batch_commit_block_1));
        assert!(batch_commits.contains(&batch_commit_block_20));

        // check that the L1 message at block 30 is deleted
        let l1_messages =
            db.get_l1_messages().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;
        assert_eq!(2, l1_messages.len());
        assert!(l1_messages.contains(&l1_message_block_1));
        assert!(l1_messages.contains(&l1_message_block_20));
    }

    #[tokio::test]
    async fn test_handle_reorg_executed_l1_messages() {
        // Instantiate indexer and db
        let (mut indexer, _database) = setup_test_indexer().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 8192];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a 3 random batch inputs and set their block numbers
        let batch_commit_block_1 =
            BatchCommitData { block_number: 5, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_commit_block_20 =
            BatchCommitData { block_number: 10, ..Arbitrary::arbitrary(&mut u).unwrap() };

        // Index batch inputs
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
        indexer.handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_20.clone()));
        for _ in 0..2 {
            let _event = indexer.next().await.unwrap().unwrap();
        }

        let batch_1 = BatchInfo::new(batch_commit_block_1.index, batch_commit_block_1.hash);
        let batch_20 = BatchInfo::new(batch_commit_block_20.index, batch_commit_block_20.hash);

        const UNITS_FOR_TESTING: u64 = 20;
        const L1_MESSAGES_NOT_EXECUTED_COUNT: u64 = 7;
        let mut l1_messages = Vec::with_capacity(UNITS_FOR_TESTING as usize);
        for l1_message_queue_index in 0..UNITS_FOR_TESTING {
            let l1_message = L1MessageEnvelope {
                queue_hash: None,
                l1_block_number: l1_message_queue_index,
                l2_block_number: (UNITS_FOR_TESTING - l1_message_queue_index >
                    L1_MESSAGES_NOT_EXECUTED_COUNT)
                    .then_some(l1_message_queue_index),
                transaction: TxL1Message {
                    queue_index: l1_message_queue_index,
                    ..Arbitrary::arbitrary(&mut u).unwrap()
                },
            };
            indexer.handle_l1_notification(L1Notification::L1Message {
                message: l1_message.transaction.clone(),
                block_number: l1_message.l1_block_number,
                block_timestamp: 0,
            });
            indexer.next().await.unwrap().unwrap();
            l1_messages.push(l1_message);
        }

        let mut blocks = Vec::with_capacity(UNITS_FOR_TESTING as usize);
        for block_number in 0..UNITS_FOR_TESTING {
            let l2_block = L2BlockInfoWithL1Messages {
                block_info: BlockInfo {
                    number: block_number,
                    hash: Arbitrary::arbitrary(&mut u).unwrap(),
                },
                l1_messages: (UNITS_FOR_TESTING - block_number > L1_MESSAGES_NOT_EXECUTED_COUNT)
                    .then_some(vec![l1_messages[block_number as usize].transaction.tx_hash()])
                    .unwrap_or_default(),
            };
            let batch_info = if block_number < 5 {
                Some(batch_1)
            } else if block_number < 10 {
                Some(batch_20)
            } else {
                None
            };
            indexer.consolidate_l2_blocks(vec![l2_block.clone()], batch_info);
            indexer.next().await.unwrap().unwrap();
            blocks.push(l2_block);
        }

        // First we assert that we dont reorg the L2 or message queue hash for a higher block
        // than any of the L1 messages.
        indexer.handle_l1_notification(L1Notification::Reorg(17));
        let event = indexer.next().await.unwrap().unwrap();
        assert_eq!(
            event,
            ChainOrchestratorEvent::ChainUnwound {
                l1_block_number: 17,
                queue_index: None,
                l2_head_block_info: None,
                l2_safe_block_info: None
            }
        );

        // Reorg at block 17 which is one of the messages that has not been executed yet. No reorg
        // but we should ensure the L1 messages have been deleted.
        indexer.handle_l1_notification(L1Notification::Reorg(7));
        let event = indexer.next().await.unwrap().unwrap();

        assert_eq!(
            event,
            ChainOrchestratorEvent::ChainUnwound {
                l1_block_number: 7,
                queue_index: Some(8),
                l2_head_block_info: Some(blocks[7].block_info),
                l2_safe_block_info: Some(blocks[4].block_info)
            }
        );

        // Now reorg at block 5 which contains L1 messages that have been executed .
        indexer.handle_l1_notification(L1Notification::Reorg(3));
        let event = indexer.next().await.unwrap().unwrap();

        assert_eq!(
            event,
            ChainOrchestratorEvent::ChainUnwound {
                l1_block_number: 3,
                queue_index: Some(4),
                l2_head_block_info: Some(blocks[3].block_info),
                l2_safe_block_info: Some(BlockInfo::new(0, indexer.chain_spec.genesis_hash())),
            }
        );
    }
}
