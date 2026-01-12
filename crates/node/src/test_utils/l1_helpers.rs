//! L1 interaction helpers for test fixtures.

use super::fixture::TestFixture;
use std::{fmt::Debug, str::FromStr, sync::Arc};

use alloy_primitives::{Address, Bytes, B256, U256};
use rollup_node_primitives::{BatchCommitData, BlockInfo, ConsensusUpdate};
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;

/// Helper for managing L1 interactions in tests.
#[derive(Debug)]
pub struct L1Helper<'a> {
    fixture: &'a mut TestFixture,
    target_node_index: Option<usize>,
}

impl<'a> L1Helper<'a> {
    /// Create a new L1 helper.
    pub(crate) const fn new(fixture: &'a mut TestFixture) -> Self {
        Self { fixture, target_node_index: None }
    }

    /// Target a specific node for L1 notifications (default is all nodes).
    pub const fn for_node(mut self, index: usize) -> Self {
        self.target_node_index = Some(index);
        self
    }

    /// Send a notification that L1 is synced.
    pub async fn sync(self) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::Synced);
        self.send_to_nodes(notification).await
    }

    /// Send a new L1 block notification.
    pub async fn new_block(self, block_number: u64) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::NewBlock(BlockInfo {
            number: block_number,
            hash: B256::random(),
        }));
        self.send_to_nodes(notification).await
    }

    /// Send an L1 reorg notification.
    pub async fn reorg_to(self, block_number: u64) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::Reorg(block_number));
        self.send_to_nodes(notification).await
    }

    /// Send an L1 consensus notification.
    pub async fn signer_update(self, new_signer: Address) -> eyre::Result<()> {
        let notification =
            Arc::new(L1Notification::Consensus(ConsensusUpdate::AuthorizedSigner(new_signer)));
        self.send_to_nodes(notification).await
    }

    /// Send an L1 reorg notification.
    pub async fn batch_commit(self, calldata_path: Option<&str>, index: u64) -> eyre::Result<()> {
        let raw_calldata = calldata_path
            .map(|path| {
                Result::<_, eyre::Report>::Ok(Bytes::from_str(&std::fs::read_to_string(path)?)?)
            })
            .transpose()?
            .unwrap_or_default();
        let batch_data = BatchCommitData {
            hash: B256::random(),
            index,
            block_number: 0,
            block_timestamp: 0,
            calldata: Arc::new(raw_calldata),
            blob_versioned_hash: None,
            finalized_block_number: None,
            reverted_block_number: None,
        };

        let notification = Arc::new(L1Notification::BatchCommit {
            block_info: BlockInfo { number: 0, hash: B256::random() },
            data: batch_data,
        });
        self.send_to_nodes(notification).await
    }

    /// Create a batch commit builder for more control.
    pub fn commit_batch(self) -> BatchCommitBuilder<'a> {
        BatchCommitBuilder::new(self)
    }

    /// Create a batch finalization builder.
    pub fn finalize_batch(self) -> BatchFinalizationBuilder<'a> {
        BatchFinalizationBuilder::new(self)
    }

    /// Create a batch revert builder.
    pub fn revert_batch(self) -> BatchRevertBuilder<'a> {
        BatchRevertBuilder::new(self)
    }

    /// Send an L1 finalized block notification.
    pub async fn finalize_l1_block(self, block_number: u64) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::Finalized(block_number));
        self.send_to_nodes(notification).await
    }

    /// Create a new L1 message builder.
    pub fn add_message(self) -> L1MessageBuilder<'a> {
        L1MessageBuilder::new(self)
    }

    /// Send notification to target nodes.
    async fn send_to_nodes(&self, notification: Arc<L1Notification>) -> eyre::Result<()> {
        let nodes = if let Some(index) = self.target_node_index {
            vec![&self.fixture.nodes[index]]
        } else {
            self.fixture.nodes.iter().collect()
        };

        for node in nodes.into_iter().flatten() {
            if let Some(tx) = &node.rollup_manager_handle.l1_watcher_mock {
                tx.notification_tx.send(notification.clone()).await?;
            }
        }

        Ok(())
    }
}

/// Builder for creating L1 messages in tests.
#[derive(Debug)]
pub struct L1MessageBuilder<'a> {
    l1_helper: L1Helper<'a>,
    l1_block_number: u64,
    queue_index: u64,
    gas_limit: u64,
    to: Address,
    value: U256,
    sender: Option<Address>,
    input: Bytes,
}

impl<'a> L1MessageBuilder<'a> {
    /// Create a new L1 message builder.
    fn new(l1_helper: L1Helper<'a>) -> Self {
        Self {
            l1_helper,
            l1_block_number: 0,
            queue_index: 0,
            gas_limit: 21000,
            to: Address::random(),
            value: U256::from(1),
            sender: None,
            input: Bytes::default(),
        }
    }

    /// Set the L1 block number for this message.
    pub const fn at_block(mut self, block_number: u64) -> Self {
        self.l1_block_number = block_number;
        self
    }

    /// Set the queue index for this message.
    pub const fn queue_index(mut self, index: u64) -> Self {
        self.queue_index = index;
        self
    }

    /// Set the gas limit for this message.
    pub const fn gas_limit(mut self, limit: u64) -> Self {
        self.gas_limit = limit;
        self
    }

    /// Set the recipient address.
    pub const fn to(mut self, address: Address) -> Self {
        self.to = address;
        self
    }

    /// Set the value to send.
    pub fn value(mut self, value: impl TryInto<U256, Error: Debug>) -> Self {
        self.value = value.try_into().expect("should convert to U256");
        self
    }

    /// Set the sender address.
    pub const fn sender(mut self, address: Address) -> Self {
        self.sender = Some(address);
        self
    }

    /// Set the input data.
    pub fn input(mut self, data: Bytes) -> Self {
        self.input = data;
        self
    }

    /// Send the L1 message to the database and notify nodes.
    pub async fn send(self) -> eyre::Result<TxL1Message> {
        let sender = self.sender.unwrap_or_else(|| Address::random());

        let tx_l1_message = TxL1Message {
            queue_index: self.queue_index,
            gas_limit: self.gas_limit,
            to: self.to,
            value: self.value,
            sender,
            input: self.input,
        };

        // Send notification to nodes
        let notification = Arc::new(L1Notification::L1Message {
            message: tx_l1_message.clone(),
            block_timestamp: self.l1_block_number * 10,
            block_info: BlockInfo { number: self.l1_block_number, hash: B256::random() },
        });

        let nodes = if let Some(index) = self.l1_helper.target_node_index {
            vec![&self.l1_helper.fixture.nodes[index]]
        } else {
            self.l1_helper.fixture.nodes.iter().collect()
        };

        for node in nodes.into_iter().flatten() {
            if let Some(tx) = &node.rollup_manager_handle.l1_watcher_mock {
                tx.notification_tx.send(notification.clone()).await?;
            }
        }

        Ok(tx_l1_message)
    }
}

/// Builder for creating batch commit notifications in tests.
#[derive(Debug)]
pub struct BatchCommitBuilder<'a> {
    l1_helper: L1Helper<'a>,
    block_info: BlockInfo,
    hash: B256,
    index: u64,
    block_timestamp: u64,
    calldata: Option<Bytes>,
    calldata_path: Option<String>,
    blob_versioned_hash: Option<B256>,
}

impl<'a> BatchCommitBuilder<'a> {
    fn new(l1_helper: L1Helper<'a>) -> Self {
        Self {
            l1_helper,
            block_info: BlockInfo { number: 0, hash: B256::random() },
            hash: B256::random(),
            index: 0,
            block_timestamp: 0,
            calldata: None,
            calldata_path: None,
            blob_versioned_hash: None,
        }
    }

    /// Set the L1 block info for this batch commit.
    pub const fn at_block(mut self, block_info: BlockInfo) -> Self {
        self.block_info = block_info;
        self
    }

    /// Set the batch hash.
    pub const fn hash(mut self, hash: B256) -> Self {
        self.hash = hash;
        self
    }

    /// Set the batch index.
    pub const fn index(mut self, index: u64) -> Self {
        self.index = index;
        self
    }

    /// Set the batch block timestamp.
    pub const fn block_timestamp(mut self, timestamp: u64) -> Self {
        self.block_timestamp = timestamp;
        self
    }

    /// Set the calldata directly.
    pub fn calldata(mut self, calldata: Bytes) -> Self {
        self.calldata = Some(calldata);
        self
    }

    /// Set the calldata from a file path.
    pub fn calldata_from_file(mut self, path: impl Into<String>) -> Self {
        self.calldata_path = Some(path.into());
        self
    }

    /// Set the blob versioned hash.
    pub const fn blob_versioned_hash(mut self, hash: B256) -> Self {
        self.blob_versioned_hash = Some(hash);
        self
    }

    /// Send the batch commit notification.
    pub async fn send(self) -> eyre::Result<(B256, u64)> {
        let raw_calldata = if let Some(calldata) = self.calldata {
            calldata
        } else if let Some(path) = self.calldata_path {
            Bytes::from_str(&std::fs::read_to_string(path)?)?
        } else {
            Bytes::default()
        };

        let batch_data = BatchCommitData {
            hash: self.hash,
            index: self.index,
            block_number: self.block_info.number,
            block_timestamp: self.block_timestamp,
            calldata: Arc::new(raw_calldata),
            blob_versioned_hash: self.blob_versioned_hash,
            finalized_block_number: None,
            reverted_block_number: None,
        };

        let notification =
            Arc::new(L1Notification::BatchCommit { block_info: self.block_info, data: batch_data });

        self.l1_helper.send_to_nodes(notification).await?;
        Ok((self.hash, self.index))
    }
}

/// Builder for creating batch finalization notifications in tests.
#[derive(Debug)]
pub struct BatchFinalizationBuilder<'a> {
    l1_helper: L1Helper<'a>,
    block_info: BlockInfo,
    hash: B256,
    index: u64,
}

impl<'a> BatchFinalizationBuilder<'a> {
    fn new(l1_helper: L1Helper<'a>) -> Self {
        Self {
            l1_helper,
            block_info: BlockInfo { number: 0, hash: B256::random() },
            hash: B256::random(),
            index: 0,
        }
    }

    /// Set the L1 block info for this batch finalization.
    pub const fn at_block(mut self, block_info: BlockInfo) -> Self {
        self.block_info = block_info;
        self
    }

    /// Set the L1 block number for this batch finalization.
    pub const fn at_block_number(mut self, block_number: u64) -> Self {
        self.block_info.number = block_number;
        self
    }

    /// Set the batch hash.
    pub const fn hash(mut self, hash: B256) -> Self {
        self.hash = hash;
        self
    }

    /// Set the batch index.
    pub const fn index(mut self, index: u64) -> Self {
        self.index = index;
        self
    }

    /// Send the batch finalization notification.
    pub async fn send(self) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::BatchFinalization {
            hash: self.hash,
            index: self.index,
            block_info: self.block_info,
        });

        self.l1_helper.send_to_nodes(notification).await
    }
}

/// Builder for creating batch revert notifications in tests.
#[derive(Debug)]
pub struct BatchRevertBuilder<'a> {
    l1_helper: L1Helper<'a>,
    block_info: BlockInfo,
    hash: B256,
    index: u64,
}

impl<'a> BatchRevertBuilder<'a> {
    fn new(l1_helper: L1Helper<'a>) -> Self {
        Self {
            l1_helper,
            block_info: BlockInfo { number: 0, hash: B256::random() },
            hash: B256::random(),
            index: 0,
        }
    }

    /// Set the L1 block info for this batch revert.
    pub const fn at_block(mut self, block_info: BlockInfo) -> Self {
        self.block_info = block_info;
        self
    }

    /// Set the L1 block number for this batch revert.
    pub const fn at_block_number(mut self, block_number: u64) -> Self {
        self.block_info.number = block_number;
        self
    }

    /// Set the batch hash.
    pub const fn hash(mut self, hash: B256) -> Self {
        self.hash = hash;
        self
    }

    /// Set the batch index.
    pub const fn index(mut self, index: u64) -> Self {
        self.index = index;
        self
    }

    /// Send the batch revert notification.
    pub async fn send(self) -> eyre::Result<()> {
        use rollup_node_primitives::BatchInfo;

        let notification = Arc::new(L1Notification::BatchRevert {
            batch_info: BatchInfo { hash: self.hash, index: self.index },
            block_info: self.block_info,
        });

        self.l1_helper.send_to_nodes(notification).await
    }
}
