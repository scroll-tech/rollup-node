use super::m20220101_000001_create_batch_commit_table::BatchCommit;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add a composite index on (status, finalized_block_number, index) for the `batch_commit`
        // table.
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_status_finalized_block_number_index")
                    .col(BatchCommit::Status)
                    .col(BatchCommit::FinalizedBlockNumber)
                    .col(BatchCommit::Index)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
