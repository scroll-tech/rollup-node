use sea_orm::entity::prelude::*;
use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(BlockData::Table)
                    .if_not_exists()
                    .col(big_unsigned(BlockData::Number).primary_key())
                    .col(binary(BlockData::ExtraData))
                    .col(binary_len(BlockData::StateRoot, 32))
                    .col(binary_len_null(BlockData::Coinbase, 20))
                    .col(text_null(BlockData::Nonce))
                    .col(tiny_integer(BlockData::Difficulty))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(BlockData::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum BlockData {
    Table,
    Number,
    ExtraData,
    StateRoot,
    Coinbase,
    Nonce,
    Difficulty,
}
