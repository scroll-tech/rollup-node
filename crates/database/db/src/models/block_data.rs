use alloy_primitives::{Address, B256, U256};
use scroll_alloy_rpc_types_engine::BlockDataHint;
use sea_orm::entity::prelude::*;

/// A database model that represents extra data for the block.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "block_data")]
pub struct Model {
    #[sea_orm(primary_key)]
    number: i64,
    extra_data: Option<Vec<u8>>,
    state_root: Option<Vec<u8>>,
    coinbase: Option<Vec<u8>>,
    nonce: Option<String>,
    difficulty: Option<i8>,
}

/// The relation for the extra data model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the extra data model.
impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for BlockDataHint {
    fn from(value: Model) -> Self {
        Self {
            extra_data: value.extra_data.map(Into::into),
            state_root: value.state_root.map(|s| B256::from_slice(&s)),
            coinbase: value.coinbase.as_deref().map(Address::from_slice),
            nonce: value.nonce.map(|n| {
                u64::from_str_radix(&n, 16).expect("nonce stored as hex string in database")
            }),
            difficulty: value.difficulty.map(U256::from),
        }
    }
}
