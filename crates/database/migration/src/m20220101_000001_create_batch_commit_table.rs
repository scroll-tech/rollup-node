use sea_orm::Statement;
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
                    .col(pk_auto(BatchCommit::Id))
                    .col(big_unsigned(BatchCommit::Index).not_null())
                    .col(binary_len(BatchCommit::Hash, HASH_LENGTH).not_null().unique_key())
                    .col(big_unsigned(BatchCommit::BlockNumber))
                    .col(big_unsigned(BatchCommit::BlockTimestamp))
                    .col(binary(BatchCommit::Calldata))
                    .col(binary_len_null(BatchCommit::BlobHash, HASH_LENGTH))
                    .col(big_unsigned_null(BatchCommit::FinalizedBlockNumber))
                    .col(big_unsigned_null(BatchCommit::RevertedBlockNumber))
                    .col(string(BatchCommit::Status).not_null())
                    .to_owned(),
            )
            .await?;

        manager
            .get_connection()
            .execute(Statement::from_sql_and_values(
                manager.get_database_backend(),
                r#"
        INSERT INTO batch_commit ("index", hash, block_number, block_timestamp, calldata, blob_hash, finalized_block_number, reverted_block_number, status)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
                vec![
                    0u64.into(),
                    vec![0u8; HASH_LENGTH as usize].into(),
                    0u64.into(),
                    0u64.into(),
                    vec![].into(),
                    None::<Vec<u8>>.into(),
                    0u64.into(),
                    None::<u64>.into(),
                    "finalized".into()
                ],
            ))
            .await?;

        // Indexes:
        // ------------------------------------------------------------

        // Add index on "index" column in batch_commit table.
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_batch_index")
                    .col(BatchCommit::Index)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        // Add index on "block number" column in batch_commit table.
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_block_number")
                    .col(BatchCommit::BlockNumber)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        // Add index on "finalized block number" column in batch_commit table.
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_finalized_block_number")
                    .col(BatchCommit::FinalizedBlockNumber)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        // Add index on "reverted block number" column in batch_commit table.
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_reverted_block_number")
                    .col(BatchCommit::RevertedBlockNumber)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        // Add index on `status` for the `batch_commit` table.
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_status")
                    .col(BatchCommit::Status)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(BatchCommit::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
pub(crate) enum BatchCommit {
    Table,
    Id,
    Index,
    Hash,
    BlockNumber,
    BlockTimestamp,
    Calldata,
    BlobHash,
    FinalizedBlockNumber,
    RevertedBlockNumber,
    Status,
}
