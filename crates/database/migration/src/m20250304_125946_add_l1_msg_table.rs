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
                    .col(binary_len_null(L1Message::QueueHash, 32))
                    .col(binary_len(L1Message::Hash, 32))
                    .col(unsigned(L1Message::L1BlockNumber))
                    .col(unsigned_null(L1Message::L2BlockNumber))
                    .col(text(L1Message::GasLimit))
                    .col(binary_len(L1Message::To, 20))
                    .col(binary_len(L1Message::Value, 32))
                    .col(binary_len(L1Message::Sender, 20))
                    .col(var_binary(L1Message::Input, 1024))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_queue_hash")
                    .col(L1Message::QueueHash)
                    .table(L1Message::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_l1_message_hash")
                    .col(L1Message::Hash)
                    .table(L1Message::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_l1_block_number")
                    .col(L1Message::L1BlockNumber)
                    .table(L1Message::Table)
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
    Hash,
    L1BlockNumber,
    L2BlockNumber,
    GasLimit,
    To,
    Value,
    Sender,
    Input,
}
