use std::ops::Deref;

use rollup_node_primitives::{BatchInput as BatchInputPrimitive, BatchInputV1, BatchInputV2};
use sea_orm::{entity::prelude::*, ActiveValue, FromJsonQueryResult};

/// A database model that represents a batch input.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "batch_input")]
pub struct Model {
    #[sea_orm(primary_key)]
    index: i64,
    version: u8,
    codec_version: u8,
    hash: Vec<u8>,
    block_number: i64,
    parent_batch_header: Vec<u8>,
    #[sea_orm(column_type = "JsonBinary")]
    chunks: Chunks,
    skipped_l1_message_bitmap: Vec<u8>,
    blob_hash: Vec<u8>,
    finalized_block_number: Option<i64>,
}

/// The relation for the batch input model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the batch input model.
impl ActiveModelBehavior for ActiveModel {}

/// A wrapper for a list of chunks.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, FromJsonQueryResult,
)]
pub struct Chunks(pub Vec<Vec<u8>>);

impl Deref for Chunks {
    type Target = Vec<Vec<u8>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<BatchInputPrimitive> for ActiveModel {
    fn from(batch_input: BatchInputPrimitive) -> Self {
        match batch_input {
            BatchInputPrimitive::BatchInputDataV1(batch_input) => Self {
                index: ActiveValue::Set(
                    batch_input.batch_index.try_into().expect("index should fit in i64"),
                ),
                version: ActiveValue::Set(1),
                codec_version: ActiveValue::Set(batch_input.version as u8),
                hash: ActiveValue::Set(batch_input.batch_hash.to_vec()),
                block_number: ActiveValue::Set(
                    batch_input.block_number.try_into().expect("block number should fit in i64"),
                ),
                parent_batch_header: ActiveValue::Set(batch_input.parent_batch_header),
                chunks: ActiveValue::Set(Chunks(batch_input.chunks)),
                skipped_l1_message_bitmap: ActiveValue::Set(batch_input.skipped_l1_message_bitmap),
                blob_hash: ActiveValue::Set(vec![]),
                finalized_block_number: ActiveValue::Unchanged(None),
            },
            BatchInputPrimitive::BatchInputDataV2(batch_input) => Self {
                index: ActiveValue::Set(
                    batch_input
                        .batch_input_data
                        .batch_index
                        .try_into()
                        .expect("index should fit in i64"),
                ),
                version: ActiveValue::Set(2),
                codec_version: ActiveValue::Set(batch_input.batch_input_data.version as u8),
                hash: ActiveValue::Set(batch_input.batch_input_data.batch_hash.to_vec()),
                block_number: ActiveValue::Set(
                    batch_input
                        .batch_input_data
                        .block_number
                        .try_into()
                        .expect("block number should fit in i64"),
                ),
                parent_batch_header: ActiveValue::Set(
                    batch_input.batch_input_data.parent_batch_header,
                ),
                chunks: ActiveValue::Set(Chunks(batch_input.batch_input_data.chunks)),
                skipped_l1_message_bitmap: ActiveValue::Set(
                    batch_input.batch_input_data.skipped_l1_message_bitmap,
                ),
                blob_hash: ActiveValue::Set(batch_input.blob_hash.to_vec()),
                finalized_block_number: ActiveValue::Unchanged(None),
            },
        }
    }
}

impl From<Model> for BatchInputPrimitive {
    fn from(value: Model) -> Self {
        let chunks = value.chunks.0;
        if value.version == 1 {
            BatchInputPrimitive::BatchInputDataV1(BatchInputV1 {
                batch_index: value.index.try_into().expect("data persisted in database is valid"),
                version: value
                    .codec_version
                    .try_into()
                    .expect("data persisted in database is valid"),
                batch_hash: value
                    .hash
                    .as_slice()
                    .try_into()
                    .expect("data persisted in database is valid"),
                block_number: value
                    .block_number
                    .try_into()
                    .expect("data persisted in database is valid"),
                parent_batch_header: value.parent_batch_header,
                chunks,
                skipped_l1_message_bitmap: value.skipped_l1_message_bitmap,
            })
        } else {
            BatchInputPrimitive::BatchInputDataV2(BatchInputV2 {
                batch_input_data: BatchInputV1 {
                    batch_index: value
                        .index
                        .try_into()
                        .expect("data persisted in database is valid"),
                    version: value
                        .codec_version
                        .try_into()
                        .expect("data persisted in database is valid"),
                    batch_hash: value
                        .hash
                        .as_slice()
                        .try_into()
                        .expect("data persisted in database is valid"),
                    block_number: value
                        .block_number
                        .try_into()
                        .expect("data persisted in database is valid"),
                    parent_batch_header: value.parent_batch_header,
                    chunks,
                    skipped_l1_message_bitmap: value.skipped_l1_message_bitmap,
                },
                blob_hash: value
                    .blob_hash
                    .as_slice()
                    .try_into()
                    .expect("data persisted in database is valid"),
            })
        }
    }
}
