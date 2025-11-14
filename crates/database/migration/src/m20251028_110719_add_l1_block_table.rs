use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create the l1_block table
        manager
            .create_table(
                Table::create()
                    .table(L1Block::Table)
                    .if_not_exists()
                    .col(big_unsigned(L1Block::BlockNumber).not_null().primary_key())
                    .col(binary_len(L1Block::BlockHash, 32).not_null().unique_key())
                    .to_owned(),
            )
            .await?;

        // Add explicit indexes for fast lookups
        manager
            .create_index(
                Index::create()
                    .name("idx-l1_block-number")
                    .table(L1Block::Table)
                    .col(L1Block::BlockNumber)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-l1_block-hash")
                    .table(L1Block::Table)
                    .col(L1Block::BlockHash)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(L1Block::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum L1Block {
    Table,
    BlockNumber,
    BlockHash,
}
