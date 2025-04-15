use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(L1Message::Table)
                    .if_not_exists()
                    .col(pk_auto(L1Message::QueueIndex))
                    .col(binary_len(L1Message::QueueHash, 32))
                    .col(unsigned(L1Message::BlockNumber))
                    .col(text(L1Message::GasLimit))
                    .col(binary_len(L1Message::To, 20))
                    .col(binary_len(L1Message::Value, 32))
                    .col(binary_len(L1Message::Sender, 20))
                    .col(var_binary(L1Message::Input, 1024))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(L1Message::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum L1Message {
    Table,
    QueueIndex,
    QueueHash,
    BlockNumber,
    GasLimit,
    To,
    Value,
    Sender,
    Input,
}
