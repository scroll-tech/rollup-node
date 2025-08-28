use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

/// TODO: remove this once we deprecated l2geth.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(BlockSignature::Table)
                    .if_not_exists()
                    .col(binary_len(BlockSignature::BlockHash, 32).primary_key())
                    .col(binary_len(BlockSignature::Signature, 65))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(BlockSignature::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
pub enum BlockSignature {
    Table,
    BlockHash,
    Signature,
}
