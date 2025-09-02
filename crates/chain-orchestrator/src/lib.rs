//! A library responsible for orchestrating the L2 chain based on data received from L1 and over the
//! L2 p2p network.

use alloy_consensus::Header;
use alloy_eips::{BlockHashOrNumber, Encodable2718};
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
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_network::Scroll;
use scroll_db::{Database, DatabaseError, DatabaseOperations, L1MessageStart, UnwindResult};
use scroll_network::NewBlockWithPeer;
use std::{
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Instant,
};
use strum::IntoEnumIterator;
use tokio::sync::Mutex;

mod action;
use action::{ChainOrchestratorFuture, PendingChainOrchestratorFuture};

mod event;
pub use event::ChainOrchestratorEvent;

mod error;
pub use error::ChainOrchestratorError;

mod metrics;
pub use metrics::{ChainOrchestratorItem, ChainOrchestratorMetrics};

/// The mask used to mask the L1 message queue hash.
const L1_MESSAGE_QUEUE_HASH_MASK: B256 =
    b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000");

type Chain = BoundedVec<Header>;

/// The [`ChainOrchestrator`] is responsible for orchestrating the progression of the L2 chain
/// based on data consolidated from L1 and the data received over the p2p network.
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
    pending_futures: VecDeque<ChainOrchestratorFuture>,
    /// The block number of the L1 finalized block.
    l1_finalized_block_number: Arc<AtomicU64>,
    /// The chain specification for the chain orchestrator.
    chain_spec: Arc<ChainSpec>,
    /// The metrics for the chain orchestrator.
    metrics: HashMap<ChainOrchestratorItem, ChainOrchestratorMetrics>,
    /// A boolean to represent if the [`ChainOrchestrator`] is in optimistic mode.
    optimistic_mode: Arc<Mutex<bool>>,
    /// The threshold for optimistic sync. If the received block is more than this many blocks
    /// ahead of the current chain, we optimistically sync the chain.
    optimistic_sync_threshold: u64,
    /// The size of the in-memory chain buffer.
    chain_buffer_size: usize,
    /// A boolean to represent if the L1 has been synced.
    l1_synced: bool,
    /// The L1 message index at which queue hashes should be computed.
    l1_message_queue_index_boundary: u64,
    /// The waker to notify when the engine driver should be polled.
    waker: AtomicWaker,
}

impl<
        ChainSpec: ScrollHardforks + EthChainSpec + Send + Sync + 'static,
        BC: BlockClient<Block = ScrollBlock> + Send + Sync + 'static,
        P: Provider<Scroll> + 'static,
    > ChainOrchestrator<ChainSpec, BC, P>
{
    /// Creates a new chain orchestrator.
    pub async fn new(
        database: Arc<Database>,
        chain_spec: Arc<ChainSpec>,
        block_client: BC,
        l2_client: P,
        optimistic_sync_threshold: u64,
        chain_buffer_size: usize,
        l1_message_queue_index_boundary: u64,
    ) -> Result<Self, ChainOrchestratorError> {
        let chain = init_chain_from_db(&database, &l2_client, chain_buffer_size).await?;
        Ok(Self {
            network_client: Arc::new(block_client),
            l2_client: Arc::new(l2_client),
            chain: Arc::new(Mutex::new(chain)),
            database,
            pending_futures: Default::default(),
            l1_finalized_block_number: Arc::new(AtomicU64::new(0)),
            chain_spec,
            metrics: ChainOrchestratorItem::iter()
                .map(|i| {
                    let label = i.as_str();
                    (i, ChainOrchestratorMetrics::new_with_labels(&[("item", label)]))
                })
                .collect(),
            optimistic_mode: Arc::new(Mutex::new(false)),
            optimistic_sync_threshold,
            chain_buffer_size,
            l1_synced: false,
            l1_message_queue_index_boundary,
            waker: AtomicWaker::new(),
        })
    }

    /// Returns the number of pending futures.
    pub fn pending_futures_len(&self) -> usize {
        self.pending_futures.len()
    }

    /// Wraps a pending chain orchestrator future, metering the completion of it.
    pub fn handle_metered(
        &mut self,
        item: ChainOrchestratorItem,
        chain_orchestrator_fut: PendingChainOrchestratorFuture,
    ) -> PendingChainOrchestratorFuture {
        let metric = self.metrics.get(&item).expect("metric exists").clone();
        let fut_wrapper = Box::pin(async move {
            let now = Instant::now();
            let res = chain_orchestrator_fut.await;
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
        let ctx = HandleBlockContext {
            chain: self.chain.clone(),
            l2_client: self.l2_client.clone(),
            optimistic_mode: self.optimistic_mode.clone(),
            optimistic_sync_threshold: self.optimistic_sync_threshold,
            network_client: self.network_client.clone(),
            database: self.database.clone(),
            chain_buffer_size: self.chain_buffer_size,
        };

        let fut = self.handle_metered(
            ChainOrchestratorItem::NewBlock,
            Box::pin(async move {
                Self::do_handle_block_from_peer(ctx, block_with_peer).await.map(Into::into)
            }),
        );
        self.pending_futures.push_back(ChainOrchestratorFuture::HandleL2Block(fut));
        self.waker.wake();
    }

    /// Handles a sequenced block.
    pub fn handle_sequenced_block(&mut self, block_with_peer: NewBlockWithPeer) {
        tracing::trace!(
            target: "scroll::chain_orchestrator",
            "Handling sequenced block {:?}",
            Into::<BlockInfo>::into(&block_with_peer.block)
        );
        let ctx = HandleBlockContext {
            chain: self.chain.clone(),
            l2_client: self.l2_client.clone(),
            optimistic_mode: self.optimistic_mode.clone(),
            optimistic_sync_threshold: self.optimistic_sync_threshold,
            network_client: self.network_client.clone(),
            database: self.database.clone(),
            chain_buffer_size: self.chain_buffer_size,
        };

        let fut = self.handle_metered(
            ChainOrchestratorItem::NewBlock,
            Box::pin(async move {
                Self::do_handle_sequenced_block(ctx, block_with_peer).await.map(Into::into)
            }),
        );
        self.pending_futures.push_back(ChainOrchestratorFuture::HandleL2Block(fut));
        self.waker.wake();
    }

    /// Handles a sequenced block by inserting it into the database and returning an event.
    async fn do_handle_sequenced_block(
        ctx: HandleBlockContext<BC, P>,
        block_with_peer: NewBlockWithPeer,
    ) -> Result<ChainOrchestratorEvent, ChainOrchestratorError> {
        let database = ctx.database.clone();
        let block_info: L2BlockInfoWithL1Messages = (&block_with_peer.block).into();
        Self::do_handle_block_from_peer(ctx, block_with_peer).await?;
        database.update_l1_messages_with_l2_block(block_info.clone()).await?;
        Ok(ChainOrchestratorEvent::L2ChainCommitted(block_info, None, true))
    }

    /// Handles a new block received from the network.
    async fn do_handle_block_from_peer(
        ctx: HandleBlockContext<BC, P>,
        block_with_peer: NewBlockWithPeer,
    ) -> Result<ChainOrchestratorEvent, ChainOrchestratorError> {
        let HandleBlockContext {
            chain,
            l2_client,
            optimistic_mode,
            optimistic_sync_threshold,
            network_client,
            database,
            chain_buffer_size,
        } = ctx;
        let NewBlockWithPeer { block: received_block, peer_id, signature } = block_with_peer;
        let mut current_chain_headers = chain.lock().await.clone().into_inner();
        let max_block_number = current_chain_headers.back().expect("chain can not be empty").number;
        let min_block_number =
            current_chain_headers.front().expect("chain can not be empty").number;

        // If the received block has a block number that is greater than the tip
        // of the chain by the optimistic sync threshold, we optimistically sync the chain and
        // update the in-memory buffer to represent the optimistic chain.
        if (received_block.header.number.saturating_sub(max_block_number)) >=
            optimistic_sync_threshold
        {
            // fetch the latest `chain_buffer_size` headers from the network for the
            // optimistic chain.
            let mut optimistic_headers = Chain::new(chain_buffer_size);
            optimistic_headers.push_front(received_block.header.clone());
            while optimistic_headers.len() < chain_buffer_size &&
                optimistic_headers.first().expect("chain can not be empty").number != 0
            {
                tracing::trace!(target: "scroll::chain_orchestrator", number = ?(optimistic_headers.first().expect("chain can not be empty").number - 1), "fetching block");
                let parent_hash =
                    optimistic_headers.first().expect("chain can not be empty").parent_hash;
                let header = network_client
                    .get_header(BlockHashOrNumber::Hash(parent_hash))
                    .await?
                    .into_data()
                    .ok_or(ChainOrchestratorError::MissingBlockHeader { hash: parent_hash })?;
                optimistic_headers.push_front(header);
            }

            *chain.lock().await = optimistic_headers;
            *optimistic_mode.lock().await = true;
            return Ok(ChainOrchestratorEvent::OptimisticSync(received_block));
        }

        // Check if we have already have this block in memory.
        if received_block.number <= max_block_number &&
            received_block.number >= min_block_number &&
            current_chain_headers.iter().any(|h| h == &received_block.header)
        {
            tracing::debug!(target: "scroll::chain_orchestrator", block_hash = ?received_block.header.hash_slow(), "block already in chain");
            return Ok(ChainOrchestratorEvent::BlockAlreadyKnown(
                received_block.header.hash_slow(),
                peer_id,
            ));
        }

        // If we are in optimistic mode and the received block has a number that is less than the
        // oldest block we have in the in-memory chain we return an event signalling we have
        // insufficient data to process the received block. This is an edge case.
        if *optimistic_mode.lock().await && (received_block.header.number <= min_block_number) {
            return Ok(ChainOrchestratorEvent::InsufficientDataForReceivedBlock(
                received_block.header.hash_slow(),
            ));
        };

        // We fetch headers for the received chain until we can reconcile it with the chain we
        // currently have in-memory.
        let mut received_chain_headers = VecDeque::from(vec![received_block.header.clone()]);

        // We should never have a re-org that is deeper than the current safe head.
        let (latest_safe_block, _) =
            database.get_latest_safe_l2_info().await?.expect("safe block must exist");

        // We search for the re-org index in the in-memory chain.
        const BATCH_FETCH_SIZE: usize = 50;
        let reorg_index = loop {
            // If we are in optimistic mode and the received chain can not be reconciled with the
            // in-memory chain we break. We will reconcile after optimistic sync has completed.
            if *optimistic_mode.lock().await &&
                received_chain_headers.front().expect("chain can not be empty").number <
                    current_chain_headers.front().expect("chain can not be empty").number
            {
                return Ok(ChainOrchestratorEvent::InsufficientDataForReceivedBlock(
                    received_block.hash_slow(),
                ));
            }

            // If the current header block number is less than the latest safe block number then
            // we should error.
            if received_chain_headers.front().expect("chain can not be empty").number <=
                latest_safe_block.number
            {
                return Err(ChainOrchestratorError::L2SafeBlockReorgDetected);
            }

            // If the received header tail has a block number that is less than the current header
            // tail then we should fetch more headers for the current chain to aid
            // reconciliation.
            if received_chain_headers.front().expect("chain can not be empty").number <
                current_chain_headers.front().expect("chain can not be empty").number
            {
                for _ in 0..BATCH_FETCH_SIZE {
                    if current_chain_headers
                        .front()
                        .expect("chain can not be empty")
                        .number
                        .saturating_sub(1) <=
                        latest_safe_block.number
                    {
                        tracing::info!(target: "scroll::chain_orchestrator", hash = %latest_safe_block.hash, number = %latest_safe_block.number, "reached safe block number for current chain - terminating fetching.");
                        break;
                    }
                    tracing::trace!(target: "scroll::chain_orchestrator", number = ?(current_chain_headers.front().expect("chain can not be empty").number - 1), "fetching block for current chain");
                    if let Some(block) = l2_client
                        .get_block_by_hash(
                            current_chain_headers
                                .front()
                                .expect("chain can not be empty")
                                .parent_hash,
                        )
                        .await?
                    {
                        let header = block.into_consensus_header();
                        current_chain_headers.push_front(header.clone());
                    } else {
                        return Err(ChainOrchestratorError::MissingBlockHeader {
                            hash: current_chain_headers
                                .front()
                                .expect("chain can not be empty")
                                .parent_hash,
                        });
                    }
                }
            }

            // We search the in-memory chain to see if we can reconcile the block import.
            if let Some(pos) = current_chain_headers.iter().rposition(|h| {
                h.hash_slow() ==
                    received_chain_headers.front().expect("chain can not be empty").parent_hash
            }) {
                // If the received fork is older than the current chain, we return an event
                // indicating that we have received an old fork.
                if (pos < current_chain_headers.len() - 1) &&
                    current_chain_headers.get(pos + 1).expect("chain can not be empty").timestamp >
                        received_chain_headers
                            .front()
                            .expect("chain can not be empty")
                            .timestamp
                {
                    return Ok(ChainOrchestratorEvent::OldForkReceived {
                        headers: received_chain_headers.into(),
                        peer_id,
                        signature,
                    });
                }
                break pos;
            }

            tracing::trace!(target: "scroll::chain_orchestrator", number = ?(received_chain_headers.front().expect("chain can not be empty").number - 1), "fetching block");
            if let Some(header) = network_client
                .get_header(BlockHashOrNumber::Hash(
                    received_chain_headers.front().expect("chain can not be empty").parent_hash,
                ))
                .await?
                .into_data()
            {
                received_chain_headers.push_front(header.clone());
            } else {
                return Err(ChainOrchestratorError::MissingBlockHeader {
                    hash: received_chain_headers
                        .front()
                        .expect("chain can not be empty")
                        .parent_hash,
                });
            }
        };

        // Fetch the blocks associated with the new chain headers.
        let new_blocks = if received_chain_headers.len() == 1 {
            vec![received_block]
        } else {
            fetch_blocks_from_network(received_chain_headers.clone().into(), network_client.clone())
                .await
        };

        // If we are not in optimistic mode, we validate the L1 messages in the new blocks.
        if !*optimistic_mode.lock().await {
            validate_l1_messages(&new_blocks, database.clone()).await?;
        }

        match reorg_index {
            // If this is a simple chain extension, we can just extend the in-memory chain and emit
            // a ChainExtended event.
            position if position == current_chain_headers.len() - 1 => {
                // Update the chain with the new blocks.
                current_chain_headers.extend(new_blocks.iter().map(|b| b.header.clone()));
                let mut new_chain = Chain::new(chain_buffer_size);
                new_chain.extend(current_chain_headers);
                *chain.lock().await = new_chain;

                Ok(ChainOrchestratorEvent::ChainExtended(ChainImport::new(
                    new_blocks, peer_id, signature,
                )))
            }
            // If we are re-organizing the in-memory chain, we need to split the chain at the reorg
            // point and extend it with the new blocks.
            position => {
                // reorg the in-memory chain to the new chain and issue a reorg event.
                let mut new_chain = Chain::new(chain_buffer_size);
                new_chain.extend(current_chain_headers.iter().take(position).cloned());
                new_chain.extend(received_chain_headers);
                *chain.lock().await = new_chain;

                Ok(ChainOrchestratorEvent::ChainReorged(ChainImport::new(
                    new_blocks, peer_id, signature,
                )))
            }
        }
    }

    /// Persist L1 consolidate blocks in the database.
    pub fn persist_l1_consolidated_blocks(
        &mut self,
        block_infos: Vec<L2BlockInfoWithL1Messages>,
        batch_info: BatchInfo,
    ) {
        let database = self.database.clone();
        let fut = self.handle_metered(
            ChainOrchestratorItem::InsertConsolidatedL2Blocks,
            Box::pin(async move {
                let head = block_infos.last().expect("block info must not be empty").clone();
                for block in block_infos {
                    database.insert_block(block, batch_info).await?;
                }
                Result::<_, ChainOrchestratorError>::Ok(Some(
                    ChainOrchestratorEvent::L2ConsolidatedBlockCommitted(head),
                ))
            }),
        );

        self.pending_futures.push_back(ChainOrchestratorFuture::HandleDerivedBlock(fut));
        self.waker.wake();
    }

    /// Consolidates L2 blocks from the network which have been validated
    pub fn consolidate_validated_l2_blocks(&mut self, block_info: Vec<L2BlockInfoWithL1Messages>) {
        let database = self.database.clone();
        let l1_synced = self.l1_synced;
        let optimistic_mode = self.optimistic_mode.clone();
        let chain = self.chain.clone();
        let l2_client = self.l2_client.clone();
        let chain_buffer_size = self.chain_buffer_size;
        let fut = self.handle_metered(
            ChainOrchestratorItem::InsertL2Block,
            Box::pin(async move {
                // If we are in optimistic mode and the L1 is synced, we consolidate the chain
                // and disable optimistic mode to enter consolidated mode
                // (consolidated_mode = !optimistic_mode).
                let consolidated = if !*optimistic_mode.lock().await {
                    true
                } else if l1_synced && *optimistic_mode.lock().await {
                    consolidate_chain(
                        database.clone(),
                        block_info.clone(),
                        chain,
                        l2_client,
                        chain_buffer_size,
                    )
                    .await?;
                    *optimistic_mode.lock().await = false;
                    true
                } else {
                    false
                };

                // Insert the blocks into the database.
                let head = block_info.last().expect("block info must not be empty").clone();
                database.update_l1_messages_from_l2_blocks(block_info).await?;

                Result::<_, ChainOrchestratorError>::Ok(Some(
                    ChainOrchestratorEvent::L2ChainCommitted(head, None, consolidated),
                ))
            }),
        );

        self.pending_futures.push_back(ChainOrchestratorFuture::HandleDerivedBlock(fut));
        self.waker.wake();
    }

    /// Handles an event from the L1.
    pub fn handle_l1_notification(&mut self, event: L1Notification) {
        let fut = match event {
            L1Notification::Reorg(block_number) => {
                ChainOrchestratorFuture::HandleReorg(self.handle_metered(
                    ChainOrchestratorItem::L1Reorg,
                    Box::pin(Self::handle_l1_reorg(
                        self.database.clone(),
                        self.chain_spec.clone(),
                        block_number,
                        self.l2_client.clone(),
                        self.chain.clone(),
                    )),
                ))
            }
            L1Notification::NewBlock(_) | L1Notification::Consensus(_) => return,
            L1Notification::Finalized(block_number) => {
                ChainOrchestratorFuture::HandleFinalized(self.handle_metered(
                    ChainOrchestratorItem::L1Finalization,
                    Box::pin(Self::handle_finalized(
                        self.database.clone(),
                        block_number,
                        self.l1_finalized_block_number.clone(),
                    )),
                ))
            }
            L1Notification::BatchCommit(batch) => {
                ChainOrchestratorFuture::HandleBatchCommit(self.handle_metered(
                    ChainOrchestratorItem::BatchCommit,
                    Box::pin(Self::handle_batch_commit(self.database.clone(), batch)),
                ))
            }
            L1Notification::L1Message { message, block_number, block_timestamp: _ } => {
                ChainOrchestratorFuture::HandleL1Message(self.handle_metered(
                    ChainOrchestratorItem::L1Message,
                    Box::pin(Self::handle_l1_message(
                        self.database.clone(),
                        message,
                        block_number,
                        self.l1_message_queue_index_boundary,
                    )),
                ))
            }
            L1Notification::BatchFinalization { hash: _hash, index, block_number } => {
                ChainOrchestratorFuture::HandleBatchFinalization(self.handle_metered(
                    ChainOrchestratorItem::BatchFinalization,
                    Box::pin(Self::handle_batch_finalization(
                        self.database.clone(),
                        index,
                        block_number,
                    )),
                ))
            }
            L1Notification::Synced => {
                self.set_l1_synced_status(true);
                return
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
        l2_client: Arc<P>,
        current_chain: Arc<Mutex<Chain>>,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let txn = database.tx().await?;
        let UnwindResult { l1_block_number, queue_index, l2_head_block_number, l2_safe_block_info } =
            txn.unwind(chain_spec.genesis_hash(), l1_block_number).await?;
        txn.commit().await?;
        let l2_head_block_info = if let Some(block_number) = l2_head_block_number {
            // Fetch the block hash of the new L2 head block.
            let block_hash = l2_client
                .get_block_by_number(block_number.into())
                .await?
                .expect("L2 head block must exist")
                .header
                .hash_slow();
            // Remove all blocks in the in-memory chain that are greater than the new L2 head block.
            let mut current_chain_headers = current_chain.lock().await;
            current_chain_headers.inner_mut().retain(|h| h.number <= block_number);
            Some(BlockInfo { number: block_number, hash: block_hash })
        } else {
            None
        };
        Ok(Some(ChainOrchestratorEvent::L1Reorg {
            l1_block_number,
            queue_index,
            l2_head_block_info,
            l2_safe_block_info,
        }))
    }

    /// Handles a finalized event by updating the chain orchestrator L1 finalized block, returning
    /// the new finalized L2 chain block and the list of finalized batches.
    async fn handle_finalized(
        database: Arc<Database>,
        block_number: u64,
        l1_block_number: Arc<AtomicU64>,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        // Set the latest finalized L1 block in the database.
        database.set_latest_finalized_l1_block_number(block_number).await?;

        // Get all unprocessed batches that have been finalized by this L1 block finalization.
        let finalized_batches =
            database.fetch_and_update_unprocessed_finalized_batches(block_number).await?;

        // Update the chain orchestrator L1 block number.
        l1_block_number.store(block_number, Ordering::Relaxed);

        Ok(Some(ChainOrchestratorEvent::L1BlockFinalized(block_number, finalized_batches)))
    }

    /// Handles an L1 message by inserting it into the database.
    async fn handle_l1_message(
        database: Arc<Database>,
        l1_message: TxL1Message,
        l1_block_number: u64,
        l1_message_queue_index_boundary: u64,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let event = ChainOrchestratorEvent::L1MessageCommitted(l1_message.queue_index);

        let queue_hash = if l1_message.queue_index == l1_message_queue_index_boundary {
            let mut input = B256::default().to_vec();
            input.append(&mut l1_message.tx_hash().to_vec());
            Some(keccak256(input) & L1_MESSAGE_QUEUE_HASH_MASK)
        } else if l1_message.queue_index > l1_message_queue_index_boundary {
            let index = l1_message.queue_index - 1;
            let mut input = database
                .get_l1_message_by_index(index)
                .await?
                .map(|m| m.queue_hash)
                .ok_or(DatabaseError::L1MessageNotFound(L1MessageStart::Index(index)))?
                .unwrap_or_default()
                .to_vec();
            input.append(&mut l1_message.tx_hash().to_vec());
            Some(keccak256(input) & L1_MESSAGE_QUEUE_HASH_MASK)
        } else {
            None
        };

        let l1_message = L1MessageEnvelope::new(l1_message, l1_block_number, None, queue_hash);
        database.insert_l1_message(l1_message).await?;
        Ok(Some(event))
    }

    /// Handles a batch input by inserting it into the database.
    async fn handle_batch_commit(
        database: Arc<Database>,
        batch: BatchCommitData,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        let txn = database.tx().await?;
        let prev_batch_index = batch.index - 1;

        // remove any batches with an index greater than the previous batch.
        let affected = txn.delete_batches_gt_batch_index(prev_batch_index).await?;

        // handle the case of a batch revert.
        let new_safe_head = if affected > 0 {
            txn.delete_l2_blocks_gt_batch_index(prev_batch_index).await?;
            txn.get_highest_block_for_batch_index(prev_batch_index).await?
        } else {
            None
        };

        let event = ChainOrchestratorEvent::BatchCommitIndexed {
            batch_info: BatchInfo::new(batch.index, batch.hash),
            l1_block_number: batch.block_number,
            safe_head: new_safe_head,
        };

        // insert the batch and commit the transaction.
        txn.insert_batch(batch).await?;
        txn.commit().await?;

        Ok(Some(event))
    }

    /// Handles a batch finalization event by updating the batch input in the database.
    async fn handle_batch_finalization(
        database: Arc<Database>,
        batch_index: u64,
        block_number: u64,
    ) -> Result<Option<ChainOrchestratorEvent>, ChainOrchestratorError> {
        // finalize all batches up to `batch_index`.
        database.finalize_batches_up_to_index(batch_index, block_number).await?;

        Ok(None)
    }
}

async fn init_chain_from_db<P: Provider<Scroll> + 'static>(
    database: &Arc<Database>,
    l2_client: &P,
    chain_buffer_size: usize,
) -> Result<BoundedVec<Header>, ChainOrchestratorError> {
    let blocks = {
        let mut blocks = Vec::with_capacity(chain_buffer_size);
        let mut blocks_stream = database.get_l2_blocks().await?.take(chain_buffer_size);
        while let Some(block_info) = blocks_stream.try_next().await? {
            let header = l2_client
                .get_block_by_hash(block_info.hash)
                .await?
                .ok_or(ChainOrchestratorError::L2BlockNotFoundInL2Client(block_info.number))?
                .header
                .into_consensus();
            blocks.push(header);
        }
        blocks.reverse();
        blocks
    };
    let mut chain: Chain = Chain::new(chain_buffer_size);
    chain.extend(blocks);
    Ok(chain)
}

impl<
        ChainSpec: ScrollHardforks + 'static,
        BC: BlockClient<Block = ScrollBlock> + Send + Sync + 'static,
        P: Provider<Scroll> + Send + Sync + 'static,
    > Stream for ChainOrchestrator<ChainSpec, BC, P>
{
    type Item = Result<ChainOrchestratorEvent, ChainOrchestratorError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Register the waker such that we can wake when required.
        self.waker.register(cx.waker());

        // Remove and poll the next future in the queue
        while let Some(mut action) = self.pending_futures.pop_front() {
            match action.poll(cx) {
                Poll::Ready(result) => match result {
                    Ok(None) => {}
                    Ok(Some(event)) => return Poll::Ready(Some(Ok(event))),
                    Err(e) => return Poll::Ready(Some(Err(e))),
                },
                Poll::Pending => {
                    self.pending_futures.push_front(action);
                    return Poll::Pending
                }
            };
        }

        Poll::Pending
    }
}

struct HandleBlockContext<BC, P> {
    pub chain: Arc<Mutex<Chain>>,
    pub l2_client: Arc<P>,
    pub optimistic_mode: Arc<Mutex<bool>>,
    pub optimistic_sync_threshold: u64,
    pub network_client: Arc<BC>,
    pub database: Arc<Database>,
    pub chain_buffer_size: usize,
}

/// Consolidates the chain by reconciling the in-memory chain with the L2 client and database.
/// This is used to ensure that the in-memory chain is consistent with the L2 chain.
async fn consolidate_chain<P: Provider<Scroll> + 'static>(
    database: Arc<Database>,
    validated_chain: Vec<L2BlockInfoWithL1Messages>,
    current_chain: Arc<Mutex<Chain>>,
    l2_client: P,
    chain_buffer_size: usize,
) -> Result<(), ChainOrchestratorError> {
    // Take the current in memory chain.
    let chain = current_chain.lock().await.clone();

    // Find highest common ancestor between the in-memory chain and the validated chain import.
    let hca_index = chain.iter().rposition(|h| {
        let h_hash = h.hash_slow();
        validated_chain.iter().any(|b| b.block_info.hash == h_hash)
    });

    // If we do not have a common ancestor this means that the chain has reorged recently and the
    // validated chain import is no longer valid. This case should be very rare. If this occurs we
    // return an error and wait for the next validated block import to reconcile the chain. This is
    // more a safety check to ensure that we do not accidentally consolidate a chain that is not
    // part of the in-memory chain.
    if hca_index.is_none() {
        // If we do not have a common ancestor, we return an error.
        *current_chain.lock().await = chain;
        return Err(ChainOrchestratorError::ChainInconsistency);
    }

    // From this point on we are no longer interested in the validated chain import as we have
    // already concluded it is part of the in-memory chain. The remainder of this function is
    // concerned with reconciling the in-memory chain with the safe head determined from L1
    // consolidation.

    // Fetch the safe head from the database. We use this as a trust anchor to reconcile the chain
    // back to.
    let safe_head = database.get_latest_safe_l2_info().await?.expect("safe head must exist").0;

    // If the in-memory chain contains the safe head, we check if the safe hash from the
    // database (L1 consolidation) matches the in-memory value. If it does not match, we return an
    // error as the in-memory chain is a fork that does not respect L1 consolidated data. This edge
    // case should not happen unless the sequencer is trying to reorg a safe block.
    let in_mem_safe_hash =
        chain.iter().find(|b| b.number == safe_head.number).map(|b| b.hash_slow());
    if let Some(in_mem_safe_hash) = in_mem_safe_hash {
        if in_mem_safe_hash != safe_head.hash {
            // If we did not consolidate back to the safe head, we return an error.
            *current_chain.lock().await =
                init_chain_from_db(&database, &l2_client, chain_buffer_size).await?;

            return Err(ChainOrchestratorError::ChainInconsistency);
        }
    };

    let mut blocks_to_consolidate = VecDeque::new();
    for header in chain.iter() {
        let block = l2_client.get_block_by_hash(header.hash_slow()).full().await.unwrap().unwrap();
        let block = block.into_consensus().map_transactions(|tx| tx.inner.into_inner());
        blocks_to_consolidate.push_back(block);
    }

    // If we do not have the safe header in the in-memory chain we should recursively fetch blocks
    // from the EN until we reach the safe block and assert that the safe head matches.
    if in_mem_safe_hash.is_none() {
        while blocks_to_consolidate.front().expect("chain can not be empty").header.number >
            safe_head.number
        {
            let parent_hash =
                blocks_to_consolidate.front().expect("chain can not be empty").header.parent_hash;
            let block = l2_client.get_block_by_hash(parent_hash).full().await.unwrap().unwrap();
            let block = block.into_consensus().map_transactions(|tx| tx.inner.into_inner());
            blocks_to_consolidate.push_front(block);
        }

        // If the safe head of the fetched chain does not match the safe head stored in database we
        // should return an error.
        if blocks_to_consolidate.front().unwrap().header.hash_slow() != safe_head.hash {
            *current_chain.lock().await =
                init_chain_from_db(&database, &l2_client, chain_buffer_size).await?;
            return Err(ChainOrchestratorError::ChainInconsistency);
        }
    }

    // TODO: modify `validate_l1_messages` to accept any type that can provide an iterator over
    // `&ScrollBlock` instead of requiring a `Vec<ScrollBlock>`.
    let blocks_to_consolidate: Vec<ScrollBlock> = blocks_to_consolidate.into_iter().collect();
    validate_l1_messages(&blocks_to_consolidate, database.clone()).await?;

    // Set the chain which has now been consolidated.
    *current_chain.lock().await = chain;

    Ok(())
}

async fn fetch_blocks_from_network<BC: BlockClient<Block = ScrollBlock> + Send + Sync + 'static>(
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

    for (header, body) in headers.into_iter().zip(bodies) {
        blocks.push(ScrollBlock::new(header, body));
    }

    blocks
}

/// Validates the L1 messages in the provided blocks against the expected L1 messages synced from
/// L1.
async fn validate_l1_messages(
    blocks: &[ScrollBlock],
    database: Arc<Database>,
) -> Result<(), ChainOrchestratorError> {
    let l1_message_hashes = blocks
        .iter()
        .flat_map(|block| {
            // Get the L1 messages from the block body.
            block
                .body
                .transactions()
                .filter(|&tx| tx.is_l1_message())
                // The hash for L1 messages is the trie hash of the transaction.
                .map(|tx| tx.trie_hash())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    // TODO: instead of using `l1_message_hashes.first().map(|tx| L1MessageStart::Hash(*tx))` to
    // determine the start of the L1 message stream, we should use a more robust method to determine
    // the start of the L1 message stream.
    let mut l1_message_stream = database
        .get_l1_messages(l1_message_hashes.first().map(|tx| L1MessageStart::Hash(*tx)))
        .await?;

    for message_hash in l1_message_hashes {
        // Get the expected L1 message from the database.
        let expected_hash = l1_message_stream.next().await.unwrap().unwrap().transaction.tx_hash();

        // If the received and expected L1 messages do not match return an error.
        if message_hash != expected_hash {
            return Err(ChainOrchestratorError::L1MessageMismatch {
                expected: expected_hash,
                actual: message_hash,
            });
        }
    }
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

    const TEST_OPTIMISTIC_SYNC_THRESHOLD: u64 = 100;
    const TEST_CHAIN_BUFFER_SIZE: usize = 2000;
    const TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY: u64 = 953885;

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

    async fn setup_test_chain_orchestrator() -> (
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
                TEST_OPTIMISTIC_SYNC_THRESHOLD,
                TEST_CHAIN_BUFFER_SIZE,
                TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY,
            )
            .await
            .unwrap(),
            db,
        )
    }

    #[tokio::test]
    async fn test_handle_commit_batch() {
        // Instantiate chain orchestrator and db
        let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let batch_commit = BatchCommitData::arbitrary(&mut u).unwrap();
        chain_orchestrator
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit.clone()));

        let event = chain_orchestrator.next().await.unwrap().unwrap();

        // Verify the event structure
        match event {
            ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, batch_commit.index);
                assert_eq!(batch_info.hash, batch_commit.hash);
                assert_eq!(safe_head, None); // No safe head since no batch revert
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        let batch_commit_result = db.get_batch_by_index(batch_commit.index).await.unwrap().unwrap();
        assert_eq!(batch_commit, batch_commit_result);
    }

    #[tokio::test]
    async fn test_handle_batch_commit_with_revert() {
        // Instantiate chain orchestrator and db
        let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Create sequential batches
        let batch_1 = BatchCommitData {
            index: 100,
            calldata: Arc::new(vec![].into()),
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let batch_2 = BatchCommitData {
            index: 101,
            calldata: Arc::new(vec![].into()),
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let batch_3 = BatchCommitData {
            index: 102,
            calldata: Arc::new(vec![].into()),
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };

        // Index first batch
        chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(batch_1.clone()));
        let event = chain_orchestrator.next().await.unwrap().unwrap();
        match event {
            ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, 100);
                assert_eq!(safe_head, None);
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        // Index second batch
        chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(batch_2.clone()));
        let event = chain_orchestrator.next().await.unwrap().unwrap();
        match event {
            ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, 101);
                assert_eq!(safe_head, None);
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        // Index third batch
        chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(batch_3.clone()));
        let event = chain_orchestrator.next().await.unwrap().unwrap();
        match event {
            ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, 102);
                assert_eq!(safe_head, None);
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        // Add some L2 blocks for the batches
        let batch_1_info = BatchInfo::new(batch_1.index, batch_1.hash);
        let batch_2_info = BatchInfo::new(batch_2.index, batch_2.hash);

        let block_1 = L2BlockInfoWithL1Messages {
            block_info: BlockInfo { number: 500, hash: Arbitrary::arbitrary(&mut u).unwrap() },
            l1_messages: vec![],
        };
        let block_2 = L2BlockInfoWithL1Messages {
            block_info: BlockInfo { number: 501, hash: Arbitrary::arbitrary(&mut u).unwrap() },
            l1_messages: vec![],
        };
        let block_3 = L2BlockInfoWithL1Messages {
            block_info: BlockInfo { number: 502, hash: Arbitrary::arbitrary(&mut u).unwrap() },
            l1_messages: vec![],
        };

        chain_orchestrator.persist_l1_consolidated_blocks(vec![block_1.clone()], batch_1_info);
        chain_orchestrator.next().await.unwrap().unwrap();

        chain_orchestrator.persist_l1_consolidated_blocks(vec![block_2.clone()], batch_2_info);
        chain_orchestrator.next().await.unwrap().unwrap();

        chain_orchestrator.persist_l1_consolidated_blocks(vec![block_3.clone()], batch_2_info);
        chain_orchestrator.next().await.unwrap().unwrap();

        // Now simulate a batch revert by submitting a new batch with index 101
        // This should delete batch 102 and any blocks associated with it
        let new_batch_2 = BatchCommitData {
            index: 101,
            calldata: Arc::new(vec![1, 2, 3].into()), // Different data
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };

        chain_orchestrator.handle_l1_notification(L1Notification::BatchCommit(new_batch_2.clone()));
        let event = chain_orchestrator.next().await.unwrap().unwrap();

        // Verify the event indicates a batch revert
        match event {
            ChainOrchestratorEvent::BatchCommitIndexed { batch_info, safe_head, .. } => {
                assert_eq!(batch_info.index, 101);
                assert_eq!(batch_info.hash, new_batch_2.hash);
                // Safe head should be the highest block from batch index <= 100
                assert_eq!(safe_head, Some(block_1.block_info));
            }
            _ => panic!("Expected BatchCommitIndexed event"),
        }

        // Verify batch 102 was deleted
        let batch_102 = db.get_batch_by_index(102).await.unwrap();
        assert!(batch_102.is_none());

        // Verify batch 101 was replaced with new data
        let updated_batch_101 = db.get_batch_by_index(101).await.unwrap().unwrap();
        assert_eq!(updated_batch_101, new_batch_2);

        // Verify batch 100 still exists
        let batch_100 = db.get_batch_by_index(100).await.unwrap();
        assert!(batch_100.is_some());
    }

    #[tokio::test]
    async fn test_handle_l1_message() {
        reth_tracing::init_test_tracing();

        // Instantiate chain orchestrator and db
        let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        let message = TxL1Message {
            queue_index: TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY - 1,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };
        let block_number = u64::arbitrary(&mut u).unwrap();
        chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
            message: message.clone(),
            block_number,
            block_timestamp: 0,
        });

        let _ = chain_orchestrator.next().await;

        let l1_message_result =
            db.get_l1_message_by_index(message.queue_index).await.unwrap().unwrap();
        let l1_message = L1MessageEnvelope::new(message, block_number, None, None);

        assert_eq!(l1_message, l1_message_result);
    }

    #[tokio::test]
    async fn test_l1_message_hash_queue() {
        // Instantiate chain orchestrator and db
        let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

        // insert the previous L1 message in database.
        chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
            message: TxL1Message {
                queue_index: TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY,
                ..Default::default()
            },
            block_number: 1475588,
            block_timestamp: 1745305199,
        });
        let _ = chain_orchestrator.next().await.unwrap().unwrap();

        // <https://sepolia.scrollscan.com/tx/0xd80cd61ac5d8665919da19128cc8c16d3647e1e2e278b931769e986d01c6b910>
        let message = TxL1Message {
            queue_index: TEST_L1_MESSAGE_QUEUE_INDEX_BOUNDARY + 1,
            gas_limit: 168000,
            to: address!("Ba50f5340FB9F3Bd074bD638c9BE13eCB36E603d"),
            value: U256::ZERO,
            sender: address!("61d8d3E7F7c656493d1d76aAA1a836CEdfCBc27b"),
            input: bytes!("8ef1332e000000000000000000000000323522a8de3cddeddbb67094eecaebc2436d6996000000000000000000000000323522a8de3cddeddbb67094eecaebc2436d699600000000000000000000000000000000000000000000000000038d7ea4c6800000000000000000000000000000000000000000000000000000000000001034de00000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000"),
        };
        chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
            message: message.clone(),
            block_number: 14755883,
            block_timestamp: 1745305200,
        });

        let _ = chain_orchestrator.next().await.unwrap().unwrap();

        let l1_message_result =
            db.get_l1_message_by_index(message.queue_index).await.unwrap().unwrap();

        assert_eq!(
            b256!("322881db10fa96b7bfed5a51a24d5a1ab86ab8fc7e0dab1b4ee4146f00000000"),
            l1_message_result.queue_hash.unwrap()
        );
    }

    #[tokio::test]
    async fn test_handle_reorg() {
        // Instantiate chain orchestrator and db
        let (mut chain_orchestrator, db) = setup_test_chain_orchestrator().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 1024];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a 3 random batch inputs and set their block numbers
        let mut batch_commit_block_1 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_1.block_number = 1;
        batch_commit_block_1.index = 1;
        let batch_commit_block_1 = batch_commit_block_1;

        let mut batch_commit_block_20 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_20.block_number = 20;
        batch_commit_block_20.index = 20;
        let batch_commit_block_20 = batch_commit_block_20;

        let mut batch_commit_block_30 = BatchCommitData::arbitrary(&mut u).unwrap();
        batch_commit_block_30.block_number = 30;
        batch_commit_block_30.index = 30;
        let batch_commit_block_30 = batch_commit_block_30;

        // Index batch inputs
        chain_orchestrator
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
        chain_orchestrator
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_20.clone()));
        chain_orchestrator
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_30.clone()));

        // Generate 3 random L1 messages and set their block numbers
        let l1_message_block_1 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 1,
            l2_block_number: None,
            transaction: TxL1Message { queue_index: 1, ..Arbitrary::arbitrary(&mut u).unwrap() },
        };
        let l1_message_block_20 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 20,
            l2_block_number: None,
            transaction: TxL1Message { queue_index: 2, ..Arbitrary::arbitrary(&mut u).unwrap() },
        };
        let l1_message_block_30 = L1MessageEnvelope {
            queue_hash: None,
            l1_block_number: 30,
            l2_block_number: None,
            transaction: TxL1Message { queue_index: 3, ..Arbitrary::arbitrary(&mut u).unwrap() },
        };

        // Index L1 messages
        chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_1.clone().transaction,
            block_number: l1_message_block_1.clone().l1_block_number,
            block_timestamp: 0,
        });
        chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_20.clone().transaction,
            block_number: l1_message_block_20.clone().l1_block_number,
            block_timestamp: 0,
        });
        chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
            message: l1_message_block_30.clone().transaction,
            block_number: l1_message_block_30.clone().l1_block_number,
            block_timestamp: 0,
        });

        // Reorg at block 20
        chain_orchestrator.handle_l1_notification(L1Notification::Reorg(20));

        for _ in 0..7 {
            chain_orchestrator.next().await.unwrap().unwrap();
        }

        // Check that the batch input at block 30 is deleted
        let batch_commits =
            db.get_batches().await.unwrap().map(|res| res.unwrap()).collect::<Vec<_>>().await;

        assert_eq!(3, batch_commits.len());
        assert!(batch_commits.contains(&batch_commit_block_1));
        assert!(batch_commits.contains(&batch_commit_block_20));

        // check that the L1 message at block 30 is deleted
        let l1_messages = db
            .get_l1_messages(None)
            .await
            .unwrap()
            .map(|res| res.unwrap())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(2, l1_messages.len());
        assert!(l1_messages.contains(&l1_message_block_1));
        assert!(l1_messages.contains(&l1_message_block_20));
    }

    // We ignore this test for now as it requires a more complex setup which leverages an L2 node
    // and is already covered in the integration test `can_handle_reorgs_while_sequencing`
    #[ignore]
    #[tokio::test]
    async fn test_handle_reorg_executed_l1_messages() {
        // Instantiate chain orchestrator and db
        let (mut chain_orchestrator, _database) = setup_test_chain_orchestrator().await;

        // Generate unstructured bytes.
        let mut bytes = [0u8; 8192];
        rand::rng().fill(bytes.as_mut_slice());
        let mut u = Unstructured::new(&bytes);

        // Generate a 3 random batch inputs and set their block numbers
        let batch_commit_block_1 =
            BatchCommitData { block_number: 5, index: 5, ..Arbitrary::arbitrary(&mut u).unwrap() };
        let batch_commit_block_10 = BatchCommitData {
            block_number: 10,
            index: 10,
            ..Arbitrary::arbitrary(&mut u).unwrap()
        };

        // Index batch inputs
        chain_orchestrator
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_1.clone()));
        chain_orchestrator
            .handle_l1_notification(L1Notification::BatchCommit(batch_commit_block_10.clone()));
        for _ in 0..2 {
            let _event = chain_orchestrator.next().await.unwrap().unwrap();
        }

        let batch_1 = BatchInfo::new(batch_commit_block_1.index, batch_commit_block_1.hash);
        let batch_10 = BatchInfo::new(batch_commit_block_10.index, batch_commit_block_10.hash);

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
            chain_orchestrator.handle_l1_notification(L1Notification::L1Message {
                message: l1_message.transaction.clone(),
                block_number: l1_message.l1_block_number,
                block_timestamp: 0,
            });
            chain_orchestrator.next().await.unwrap().unwrap();
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
                Some(batch_10)
            } else {
                None
            };
            if let Some(batch_info) = batch_info {
                chain_orchestrator
                    .persist_l1_consolidated_blocks(vec![l2_block.clone()], batch_info);
            } else {
                chain_orchestrator.consolidate_validated_l2_blocks(vec![l2_block.clone()]);
            }

            chain_orchestrator.next().await.unwrap().unwrap();
            blocks.push(l2_block);
        }

        // First we assert that we dont reorg the L2 or message queue hash for a higher block
        // than any of the L1 messages.
        chain_orchestrator.handle_l1_notification(L1Notification::Reorg(17));
        let event = chain_orchestrator.next().await.unwrap().unwrap();
        assert_eq!(
            event,
            ChainOrchestratorEvent::L1Reorg {
                l1_block_number: 17,
                queue_index: None,
                l2_head_block_info: None,
                l2_safe_block_info: None
            }
        );

        // Reorg at block 7 which is one of the messages that has not been executed yet. No reorg
        // but we should ensure the L1 messages have been deleted.
        chain_orchestrator.handle_l1_notification(L1Notification::Reorg(7));
        let event = chain_orchestrator.next().await.unwrap().unwrap();

        assert_eq!(
            event,
            ChainOrchestratorEvent::L1Reorg {
                l1_block_number: 7,
                queue_index: Some(8),
                l2_head_block_info: Some(blocks[7].block_info),
                l2_safe_block_info: Some(blocks[4].block_info)
            }
        );

        // Now reorg at block 5 which contains L1 messages that have been executed .
        chain_orchestrator.handle_l1_notification(L1Notification::Reorg(3));
        let event = chain_orchestrator.next().await.unwrap().unwrap();

        assert_eq!(
            event,
            ChainOrchestratorEvent::L1Reorg {
                l1_block_number: 3,
                queue_index: Some(4),
                l2_head_block_info: Some(blocks[3].block_info),
                l2_safe_block_info: Some(BlockInfo::new(
                    0,
                    chain_orchestrator.chain_spec.genesis_hash()
                )),
            }
        );
    }
}
