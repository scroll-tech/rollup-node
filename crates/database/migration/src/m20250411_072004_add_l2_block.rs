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
                    .table(L2Block::Table)
                    .if_not_exists()
                    .col(pk_auto(L2Block::BlockNumber))
                    .col(binary_len(L2Block::BlockHash, 32))
                    .col(big_unsigned_null(L2Block::BatchIndex))
                    .col(binary_len_null(L2Block::BatchHash, 32))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_batch_index")
                            .from(L2Block::Table, L2Block::BatchIndex)
                            .to(BatchCommit::Table, BatchCommit::Index)
                            .on_delete(ForeignKeyAction::SetNull)
                            .on_update(ForeignKeyAction::SetNull),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_batch_hash")
                            .from(L2Block::Table, L2Block::BatchHash)
                            .to(BatchCommit::Table, BatchCommit::Hash)
                            .on_delete(ForeignKeyAction::SetNull)
                            .on_update(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(L2Block::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum L2Block {
    Table,
    BatchIndex,
    BatchHash,
    BlockNumber,
    BlockHash,
}
