use alloy_primitives::{Bytes, B256, U256};
use sea_orm::{prelude::*, ActiveValue};
use sea_orm_migration::{prelude::*, seaql_migrations::Relation};

const BLOCK_DATA_CSV: &str = include_str!("../migration-data/block_data.csv");

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut rdr = csv::Reader::from_reader(BLOCK_DATA_CSV.as_bytes());

        let db = manager.get_connection();
        let records: Vec<ActiveModel> =
            rdr.deserialize::<Record>().filter_map(|a| a.ok().map(Into::into)).collect();
        // we ignore the `Failed to find inserted item` error.
        let _ = Entity::insert_many(records).exec(db).await;

        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct Record {
    number: u64,
    hash: B256,
    extra_data: Bytes,
    difficulty: U256,
}

/// This model should match the model at `scroll_db::models::extra_data`.
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "block_data")]
pub struct Model {
    #[sea_orm(primary_key)]
    number: i64,
    hash: Vec<u8>,
    extra_data: Vec<u8>,
    difficulty: Vec<u8>,
}
impl ActiveModelBehavior for ActiveModel {}

impl From<Record> for ActiveModel {
    fn from(value: Record) -> Self {
        Self {
            number: ActiveValue::Set(value.number as i64),
            hash: ActiveValue::Set(value.hash.to_vec()),
            extra_data: ActiveValue::Set(value.extra_data.to_vec()),
            difficulty: ActiveValue::Set(value.difficulty.to_be_bytes_vec()),
        }
    }
}
