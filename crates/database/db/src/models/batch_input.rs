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
        let (version, batch_input_v1, blob_hash) = match batch_input {
            BatchInputPrimitive::BatchInputDataV1(batch_input) => (1, batch_input, vec![]),
            BatchInputPrimitive::BatchInputDataV2(batch_input) => {
                (2, batch_input.batch_input_base, batch_input.blob_hash.to_vec())
            }
        };
        Self {
            index: ActiveValue::Set(
                batch_input_v1.batch_index.try_into().expect("index should fit in i64"),
            ),
            version: ActiveValue::Set(version),
            codec_version: ActiveValue::Set(batch_input_v1.version),
            hash: ActiveValue::Set(batch_input_v1.batch_hash.to_vec()),
            block_number: ActiveValue::Set(
                batch_input_v1.block_number.try_into().expect("block number should fit in i64"),
            ),
            parent_batch_header: ActiveValue::Set(batch_input_v1.parent_batch_header),
            chunks: ActiveValue::Set(Chunks(batch_input_v1.chunks)),
            skipped_l1_message_bitmap: ActiveValue::Set(batch_input_v1.skipped_l1_message_bitmap),
            blob_hash: ActiveValue::Set(blob_hash),
            finalized_block_number: ActiveValue::Unchanged(None),
        }
    }
}

impl From<Model> for BatchInputPrimitive {
    fn from(value: Model) -> Self {
        let chunks = value.chunks.0;
        let batch_input_v1 = BatchInputV1 {
            version: value.codec_version,
            batch_index: value.index.try_into().expect("data persisted in database is valid"),
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
        };

        if value.version == 1 {
            Self::BatchInputDataV1(batch_input_v1)
        } else {
            Self::BatchInputDataV2(BatchInputV2 {
                batch_input_base: batch_input_v1,
                blob_hash: value
                    .blob_hash
                    .as_slice()
                    .try_into()
                    .expect("data persisted in database is valid"),
            })
        }
    }
}
