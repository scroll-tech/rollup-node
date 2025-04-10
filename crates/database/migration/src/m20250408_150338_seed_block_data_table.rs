use std::{path::PathBuf, sync::LazyLock};

use alloy_primitives::{Bytes, B256, U256};
use sea_orm::{prelude::*, ActiveValue, TransactionTrait};
use sea_orm_migration::{prelude::*, seaql_migrations::Relation};

static BLOCK_DATA_FILE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_path.join("./migration-data/block_data.csv")
});

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let file =
            std::fs::File::open(BLOCK_DATA_FILE_PATH.clone()).expect("missing migration data");
        let mut rdr = csv::Reader::from_reader(file);

        let db = manager.get_connection();
        let transaction = db.begin().await?;
        for result in rdr.deserialize() {
            let record: Record = result.map_err(|err| DbErr::Migration(err.to_string()))?;
            let db_model: ActiveModel = record.into();
            // we ignore the `Failed to find inserted item` error.
            let _ = db_model.insert(&transaction).await;
        }

        transaction.commit().await
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
