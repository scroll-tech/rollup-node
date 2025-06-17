use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Metadata::Table)
                    .if_not_exists()
                    .col(string(Metadata::Key).primary_key())
                    .col(string(Metadata::Value))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Metadata::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum Metadata {
    Table,
    Key,
    Value,
}
