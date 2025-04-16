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
                    .table(DerivedBlock::Table)
                    .if_not_exists()
                    .col(pk_auto(DerivedBlock::BlockNumber))
                    .col(binary_len(DerivedBlock::BlockHash, 32))
                    .col(big_unsigned(DerivedBlock::BatchIndex))
                    .col(binary_len(DerivedBlock::BatchHash, 32))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_batch_index")
                            .from(DerivedBlock::Table, DerivedBlock::BatchIndex)
                            .to(BatchCommit::Table, BatchCommit::Index)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_batch_hash")
                            .from(DerivedBlock::Table, DerivedBlock::BatchHash)
                            .to(BatchCommit::Table, BatchCommit::Hash)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(DerivedBlock::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum DerivedBlock {
    Table,
    BatchIndex,
    BatchHash,
    BlockNumber,
    BlockHash,
}
