use alloy_primitives::B256;
use rollup_node_primitives::{BatchInfo, BlockInfo};
use sea_orm::{entity::prelude::*, ActiveValue};

/// A database model that represents a derived block.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "derived_block")]
pub struct Model {
    #[sea_orm(primary_key)]
    block_number: i64,
    block_hash: Vec<u8>,
    batch_index: i64,
    batch_hash: Vec<u8>,
}

impl Model {
    pub(crate) fn block_info(&self) -> BlockInfo {
        BlockInfo { number: self.block_number as u64, hash: B256::from_slice(&self.block_hash) }
    }
}

/// The relation for the batch input model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    /// A relation with the batch commit table, where column batch hash of the
    /// batch to block table belongs to the column hash of the batch commit
    /// table.
    #[sea_orm(
        belongs_to = "super::batch_commit::Entity",
        from = "Column::BatchHash",
        to = "super::batch_commit::Column::Hash"
    )]
    BatchCommit,
}

impl Related<super::batch_commit::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::BatchCommit.def()
    }
}

/// The active model behavior for the batch input model.
impl ActiveModelBehavior for ActiveModel {}

impl From<(BatchInfo, BlockInfo)> for ActiveModel {
    fn from((batch_info, block_info): (BatchInfo, BlockInfo)) -> Self {
        Self {
            batch_index: ActiveValue::Set(
                batch_info.index.try_into().expect("index should fit in i64"),
            ),
            batch_hash: ActiveValue::Set(batch_info.hash.to_vec()),
            block_number: ActiveValue::Set(
                block_info.number.try_into().expect("block number should fit in i64"),
            ),
            block_hash: ActiveValue::Set(block_info.hash.to_vec()),
        }
    }
}
