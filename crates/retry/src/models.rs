use crate::BatchCommitData;
use sea_orm::entity::prelude::*;
use sea_orm::{ActiveModelBehavior, ActiveValue, DeriveEntityModel};

/// A database model that represents a batch input.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "batch_commit")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub(crate) index: i64,
    block_number: i64,
    block_timestamp: i64,
    pub(crate) finalized_block_number: Option<i64>,
    processed: bool,
}

/// The active model behavior for the batch input model.
impl ActiveModelBehavior for ActiveModel {}

/// The relation for the extra data model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl From<Model> for BatchCommitData {
    fn from(value: Model) -> Self {
        Self {
            index: value.index as u64,
            block_number: value.block_number as u64,
            block_timestamp: value.block_timestamp as u64,
            finalized_block_number: value.finalized_block_number.map(|b| b as u64),
        }
    }
}

impl From<BatchCommitData> for ActiveModel {
    fn from(batch_commit: BatchCommitData) -> Self {
        Self {
            index: ActiveValue::Set(
                batch_commit
                    .index
                    .try_into()
                    .expect("index should fit in i64"),
            ),
            block_number: ActiveValue::Set(
                batch_commit
                    .block_number
                    .try_into()
                    .expect("block number should fit in i64"),
            ),
            block_timestamp: ActiveValue::Set(
                batch_commit
                    .block_timestamp
                    .try_into()
                    .expect("block timestamp should fit in i64"),
            ),
            finalized_block_number: ActiveValue::Unchanged(None),
            processed: ActiveValue::Unchanged(false),
        }
    }
}
