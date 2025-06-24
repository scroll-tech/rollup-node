use alloy_primitives::U256;
use scroll_alloy_rpc_types_engine::BlockDataHint;
use sea_orm::entity::prelude::*;

/// A database model that represents extra data for the block.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "block_data")]
pub struct Model {
    #[sea_orm(primary_key)]
    number: i64,
    extra_data: Vec<u8>,
    state_root: Vec<u8>,
    coinbase: Option<Vec<u8>>,
    nonce: Option<String>,
    difficulty: Vec<u8>,
}

/// The relation for the extra data model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the extra data model.
impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for BlockDataHint {
    fn from(value: Model) -> Self {
        Self {
            extra_data: value.extra_data.into(),
            difficulty: U256::from_be_slice(value.difficulty.as_ref()),
        }
    }
}
