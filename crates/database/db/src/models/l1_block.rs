use alloy_primitives::B256;
use rollup_node_primitives::BlockInfo;
use sea_orm::{entity::prelude::*, ActiveValue};

/// A database model that represents an L1 block.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "l1_block")]
pub struct Model {
    #[sea_orm(primary_key)]
    block_number: i64,
    block_hash: Vec<u8>,
}

// impl Model {
//     /// Returns the `BlockInfo` representation of this L1 block.
//     pub(crate) fn block_info(&self) -> BlockInfo {
//         BlockInfo { number: self.block_number as u64, hash: B256::from_slice(&self.block_hash) }
//     }
// }

/// The relation for the batch input model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the batch input model.
impl ActiveModelBehavior for ActiveModel {}

impl From<BlockInfo> for ActiveModel {
    fn from(block_info: BlockInfo) -> Self {
        Self {
            block_number: ActiveValue::Set(
                block_info.number.try_into().expect("block number should fit in i64"),
            ),
            block_hash: ActiveValue::Set(block_info.hash.to_vec()),
        }
    }
}

impl From<Model> for BlockInfo {
    fn from(value: Model) -> Self {
        Self { number: value.block_number as u64, hash: B256::from_slice(&value.block_hash) }
    }
}
