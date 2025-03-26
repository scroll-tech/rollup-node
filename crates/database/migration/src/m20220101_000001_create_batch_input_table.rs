use sea_orm_migration::{prelude::*, schema::*};

// TODO: migrate these to a constants module
const HASH_LENGTH: u32 = 32;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(BatchInput::Table)
                    .if_not_exists()
                    .col(pk_auto(BatchInput::Index))
                    .col(binary_len(BatchInput::Hash, HASH_LENGTH))
                    .col(big_unsigned(BatchInput::BlockNumber))
                    .col(binary(BatchInput::Calldata))
                    .col(binary_len_null(BatchInput::BlobHash, HASH_LENGTH))
                    .col(boolean_null(BatchInput::FinalizedBlockNumber))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(BatchInput::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum BatchInput {
    Table,
    Index,
    Hash,
    BlockNumber,
    Calldata,
    BlobHash,
    FinalizedBlockNumber,
}
