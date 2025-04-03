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
                    .table(BatchCommit::Table)
                    .if_not_exists()
                    .col(pk_auto(BatchCommit::Index))
                    .col(binary_len(BatchCommit::Hash, HASH_LENGTH))
                    .col(big_unsigned(BatchCommit::BlockNumber))
                    .col(big_unsigned(BatchCommit::BlockTimestamp))
                    .col(binary(BatchCommit::Calldata))
                    .col(binary_len_null(BatchCommit::BlobHash, HASH_LENGTH))
                    .col(big_unsigned_null(BatchCommit::FinalizedBlockNumber))
                    .col(boolean(BatchCommit::Processed))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(BatchCommit::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum BatchCommit {
    Table,
    Index,
    Hash,
    BlockNumber,
    BlockTimestamp,
    Calldata,
    BlobHash,
    FinalizedBlockNumber,
    Processed,
}
