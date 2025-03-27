use std::sync::Arc;

use rollup_node_primitives::BatchCommitData;
use sea_orm::{entity::prelude::*, ActiveValue};

/// A database model that represents a batch input.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "batch_commit")]
pub struct Model {
    #[sea_orm(primary_key)]
    index: i64,
    hash: Vec<u8>,
    block_number: i64,
    block_timestamp: i64,
    calldata: Vec<u8>,
    blob_hash: Option<Vec<u8>>,
    finalized_block_number: Option<i64>,
}

/// The relation for the batch input model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the batch input model.
impl ActiveModelBehavior for ActiveModel {}

impl From<BatchCommitData> for ActiveModel {
    fn from(batch_commit: BatchCommitData) -> Self {
        Self {
            index: ActiveValue::Set(
                batch_commit.index.try_into().expect("index should fit in i64"),
            ),
            hash: ActiveValue::Set(batch_commit.hash.to_vec()),
            block_number: ActiveValue::Set(
                batch_commit.block_number.try_into().expect("block number should fit in i64"),
            ),
            block_timestamp: ActiveValue::Set(
                batch_commit.block_timestamp.try_into().expect("block number should fit in i64"),
            ),
            calldata: ActiveValue::Set(batch_commit.calldata.0.to_vec()),
            blob_hash: ActiveValue::Set(batch_commit.blob_versioned_hash.map(|b| b.to_vec())),
            finalized_block_number: ActiveValue::Unchanged(None),
        }
    }
}

impl From<Model> for BatchCommitData {
    fn from(value: Model) -> Self {
        Self {
            hash: value.hash.as_slice().try_into().expect("data persisted in database is valid"),
            index: value.index as u64,
            block_number: value.block_number as u64,
            block_timestamp: value.block_timestamp as u64,
            calldata: Arc::new(value.calldata.into()),
            blob_versioned_hash: value
                .blob_hash
                .map(|b| b.as_slice().try_into().expect("data persisted in database is valid")),
        }
    }
}
