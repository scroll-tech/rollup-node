use crate::m20250304_125946_add_l1_msg_table::L1Message;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add index on `queue_index` for the `l1_message` table.
        manager
            .create_index(
                Index::create()
                    .name("idx_l1_message_queue_index")
                    .col(L1Message::QueueIndex)
                    .table(L1Message::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop index `queue_index` for the `l1_message` table.
        manager
            .drop_index(
                Index::drop().name("idx_l1_message_queue_index").table(L1Message::Table).to_owned(),
            )
            .await
    }
}
