//! L1 interaction helpers for test fixtures.

use super::fixture::TestFixture;
use std::{fmt::Debug, str::FromStr, sync::Arc};

use alloy_primitives::{Address, Bytes, B256, U256};
use rollup_node_primitives::{BatchCommitData, ConsensusUpdate};
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
    pub(crate) fn new(fixture: &'a mut TestFixture) -> Self {
        Self { fixture, target_node_index: None }
    }

    /// Target a specific node for L1 notifications (default is all nodes).
    pub const fn for_node(mut self, index: usize) -> Self {
        self.target_node_index = Some(index);
        self
    }

    /// Send a notification that L1 is synced.
    pub async fn sync(mut self) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::Synced);
        self.send_to_nodes(notification).await
    }

    /// Send a new L1 block notification.
    pub async fn new_block(mut self, block_number: u64) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::NewBlock(block_number));
        self.send_to_nodes(notification).await
    }

    /// Send an L1 reorg notification.
    pub async fn reorg_to(mut self, block_number: u64) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::Reorg(block_number));
        self.send_to_nodes(notification).await
    }

    /// Send an L1 consensus notification.
    pub async fn signer_update(mut self, new_signer: Address) -> eyre::Result<()> {
        let notification =
            Arc::new(L1Notification::Consensus(ConsensusUpdate::AuthorizedSigner(new_signer)));
        self.send_to_nodes(notification).await
    }

    /// Send an L1 reorg notification.
    pub async fn batch_commit(
        mut self,
        calldata_path: Option<&str>,
        index: u64,
    ) -> eyre::Result<()> {
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
        };

        let notification = Arc::new(L1Notification::BatchCommit(batch_data));
        self.send_to_nodes(notification).await
    }

    /// Create a new L1 message builder.
    pub fn add_message(self) -> L1MessageBuilder<'a> {
        L1MessageBuilder::new(self)
    }

    /// Send notification to target nodes.
    async fn send_to_nodes(&mut self, notification: Arc<L1Notification>) -> eyre::Result<()> {
        let nodes = if let Some(index) = self.target_node_index {
            vec![&self.fixture.nodes[index]]
        } else {
            self.fixture.nodes.iter().collect()
        };

        for node in nodes {
            if let Some(tx) = &node.l1_watcher_tx {
                tx.send(notification.clone()).await?;
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
        let sender = self.sender.unwrap_or_else(|| self.l1_helper.fixture.wallet_address());

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
            block_number: self.l1_block_number,
            block_timestamp: self.l1_block_number * 10,
        });

        let nodes = if let Some(index) = self.l1_helper.target_node_index {
            vec![&self.l1_helper.fixture.nodes[index]]
        } else {
            self.l1_helper.fixture.nodes.iter().collect()
        };

        for node in nodes {
            if let Some(tx) = &node.l1_watcher_tx {
                tx.send(notification.clone()).await?;
            }
        }

        Ok(tx_l1_message)
    }
}
