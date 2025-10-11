use super::m20250304_125946_add_l1_msg_table::L1Message;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create index for the l1 message to l2 block mapping.
        manager
            .create_index(
                Index::create()
                    .name("idx_l1_message_l2_block")
                    .col(L1Message::L2BlockNumber)
                    .table(L1Message::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop index for the l1 message to l2 block mapping.
        manager
            .drop_index(
                Index::drop().name("idx_l1_message_l2_block").table(L1Message::Table).to_owned(),
            )
            .await?;

        Ok(())
    }
}
