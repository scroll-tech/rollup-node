use super::m20220101_000001_create_batch_commit_table::BatchCommit;

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(BatchToBlock::Table)
                    .if_not_exists()
                    .col(big_unsigned(BatchToBlock::BlockNumber).primary_key())
                    .col(binary_len(BatchToBlock::BlockHash, 32))
                    .col(big_unsigned(BatchToBlock::BatchIndex))
                    .col(binary_len(BatchToBlock::BatchHash, 32))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_batch_index")
                            .from(BatchToBlock::Table, BatchToBlock::BatchIndex)
                            .to(BatchCommit::Table, BatchCommit::Index)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_batch_hash")
                            .from(BatchToBlock::Table, BatchToBlock::BatchHash)
                            .to(BatchCommit::Table, BatchCommit::Hash)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(BatchToBlock::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum BatchToBlock {
    Table,
    BatchIndex,
    BatchHash,
    BlockNumber,
    BlockHash,
}
