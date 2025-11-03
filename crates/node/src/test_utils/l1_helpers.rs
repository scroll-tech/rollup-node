//! L1 interaction helpers for test fixtures.

use super::fixture::TestFixture;
use alloy_primitives::{Address, Bytes, U256};
use rollup_node_primitives::L1MessageEnvelope;
use rollup_node_watcher::L1Notification;
use scroll_alloy_consensus::TxL1Message;
use scroll_db::DatabaseWriteOperations;
use std::{fmt::Debug, sync::Arc};

/// Helper for managing L1 interactions in tests.
#[derive(Debug)]
pub struct L1Helper<'a, EC> {
    fixture: &'a mut TestFixture<EC>,
    target_node_index: Option<usize>,
}

impl<'a, EC> L1Helper<'a, EC> {
    /// Create a new L1 helper.
    pub(crate) fn new(fixture: &'a mut TestFixture<EC>) -> Self {
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
        let notification = Arc::new(L1Notification::NewBlock(block_number));
        self.send_to_nodes(notification).await
    }

    /// Send an L1 reorg notification.
    pub async fn reorg_to(self, block_number: u64) -> eyre::Result<()> {
        let notification = Arc::new(L1Notification::Reorg(block_number));
        self.send_to_nodes(notification).await
    }

    /// Create a new L1 message builder.
    pub fn add_message(self) -> L1MessageBuilder<'a, EC> {
        L1MessageBuilder::new(self)
    }

    /// Set the latest L1 block number in the database.
    pub async fn set_latest_block(self, block_number: u64) -> eyre::Result<()> {
        if let Some(db) = &self.fixture.database {
            db.set_latest_l1_block_number(block_number).await?;
        }
        Ok(())
    }

    /// Set the finalized L1 block number in the database.
    pub async fn set_finalized_block(self, block_number: u64) -> eyre::Result<()> {
        if let Some(db) = &self.fixture.database {
            db.set_finalized_l1_block_number(block_number).await?;
        }
        Ok(())
    }

    /// Send notification to target nodes.
    async fn send_to_nodes(self, notification: Arc<L1Notification>) -> eyre::Result<()> {
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
pub struct L1MessageBuilder<'a, EC> {
    l1_helper: L1Helper<'a, EC>,
    l1_block_number: u64,
    queue_index: u64,
    gas_limit: u64,
    to: Address,
    value: U256,
    sender: Option<Address>,
    input: Bytes,
}

impl<'a, EC> L1MessageBuilder<'a, EC> {
    /// Create a new L1 message builder.
    fn new(l1_helper: L1Helper<'a, EC>) -> Self {
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

        let l1_message_envelope = L1MessageEnvelope {
            l1_block_number: self.l1_block_number,
            l2_block_number: None,
            queue_hash: None,
            transaction: tx_l1_message.clone(),
        };

        // Insert into database if available
        if let Some(db) = &self.l1_helper.fixture.database {
            db.insert_l1_message(l1_message_envelope.clone()).await?;
        }

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

    /// Insert the message into the database without notifying nodes.
    pub async fn insert_only(self) -> eyre::Result<TxL1Message> {
        let sender = self.sender.unwrap_or_else(|| self.l1_helper.fixture.wallet_address());

        let tx_l1_message = TxL1Message {
            queue_index: self.queue_index,
            gas_limit: self.gas_limit,
            to: self.to,
            value: self.value,
            sender,
            input: self.input,
        };

        let l1_message_envelope = L1MessageEnvelope {
            l1_block_number: self.l1_block_number,
            l2_block_number: None,
            queue_hash: None,
            transaction: tx_l1_message.clone(),
        };

        // Insert into database if available
        if let Some(db) = &self.l1_helper.fixture.database {
            db.insert_l1_message(l1_message_envelope).await?;
        }

        Ok(tx_l1_message)
    }
}
