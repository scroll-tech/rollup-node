use sea_orm_migration::{prelude::*, schema::*};

// TODO: migrate these to a constants module
// CONSTANTS
const BATCH_HEADER_LENGTH: u32 = 32;

const CHUNKS_LENGTH: u32 = 1024;

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
                    .col(tiny_integer(BatchInput::Version))
                    .col(tiny_integer(BatchInput::CodecVersion))
                    .col(binary_len(BatchInput::Hash, HASH_LENGTH))
                    .col(big_unsigned(BatchInput::BlockNumber))
                    .col(binary_len(BatchInput::ParentBatchHeader, BATCH_HEADER_LENGTH))
                    .col(var_binary(BatchInput::Chunks, CHUNKS_LENGTH))
                    .col(var_binary(BatchInput::SkippedL1MessageBitmap, HASH_LENGTH))
                    // TODO: Set the blob hash as nullable
                    .col(binary_len(BatchInput::BlobHash, HASH_LENGTH))
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
    Version,
    Index,
    CodecVersion,
    Hash,
    BlockNumber,
    ParentBatchHeader,
    Chunks,
    SkippedL1MessageBitmap,
    BlobHash,
    FinalizedBlockNumber,
    // TODO: Do we need the blob proof?
}
