use alloy_primitives::{Address, B256, U256};
use rollup_node_primitives::L1MessageEnvelope;
use scroll_alloy_consensus::TxL1Message;
use sea_orm::{entity::prelude::*, ActiveValue};

/// A database model that represents a L1 message.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "l1_message")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub(crate) queue_index: i64,
    queue_hash: Option<Vec<u8>>,
    hash: Vec<u8>,
    l1_block_number: i64,
    gas_limit: String,
    to: Vec<u8>,
    value: Vec<u8>,
    sender: Vec<u8>,
    input: Vec<u8>,
    pub(crate) l2_block_number: Option<i64>,
}

/// The relation for the L1 message model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the L1 message model.
impl ActiveModelBehavior for ActiveModel {}

impl From<L1MessageEnvelope> for ActiveModel {
    fn from(value: L1MessageEnvelope) -> Self {
        Self {
            queue_index: ActiveValue::Set(value.transaction.queue_index as i64),
            queue_hash: ActiveValue::Set(value.queue_hash.map(|q| q.to_vec())),
            hash: ActiveValue::Set(value.transaction.tx_hash().to_vec()),
            l1_block_number: ActiveValue::Set(value.l1_block_number as i64),
            gas_limit: ActiveValue::Set(value.transaction.gas_limit.to_string()),
            to: ActiveValue::Set(value.transaction.to.to_vec()),
            value: ActiveValue::Set(value.transaction.value.to_le_bytes_vec()),
            sender: ActiveValue::Set(value.transaction.sender.to_vec()),
            input: ActiveValue::Set(value.transaction.input.to_vec()),
            l2_block_number: ActiveValue::Set(value.l2_block_number.map(|b| b as i64)),
        }
    }
}

impl From<Model> for L1MessageEnvelope {
    fn from(value: Model) -> Self {
        Self {
            l1_block_number: value.l1_block_number as u64,
            l2_block_number: value.l2_block_number.map(|b| b as u64),
            queue_hash: value.queue_hash.map(|q| B256::from_slice(&q)),
            transaction: TxL1Message {
                queue_index: value.queue_index as u64,
                gas_limit: value.gas_limit.parse().expect("gas limit is valid"),
                to: Address::from_slice(&value.to),
                value: U256::from_le_slice(&value.value),
                sender: Address::from_slice(&value.sender),
                input: value.input.into(),
            },
        }
    }
}
