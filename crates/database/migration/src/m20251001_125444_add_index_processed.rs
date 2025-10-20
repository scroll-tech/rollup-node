use crate::m20220101_000001_create_batch_commit_table::BatchCommit;

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add index on `processed` for the `batch_commit` table.
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_processed")
                    .col(BatchCommit::Processed)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop index `processed` for the `batch_commit` table.
        manager
            .drop_index(
                Index::drop()
                    .name("idx_batch_commit_processed")
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
