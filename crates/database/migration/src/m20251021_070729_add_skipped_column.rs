use crate::m20250304_125946_add_l1_msg_table::L1Message;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add the `skipped` column to the `l1_message` table.
        manager
            .alter_table(
                Table::alter()
                    .table(L1Message::Table)
                    .add_column(
                        ColumnDef::new(L1Message::Skipped).boolean().not_null().default(false),
                    )
                    .to_owned(),
            )
            .await?;

        // Add index on `skipped` for the `l1_message` table.
        manager
            .create_index(
                Index::create()
                    .name("idx_l1_message_skipped")
                    .col(L1Message::Skipped)
                    .table(L1Message::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // drop the `skipped` column on the `l1_message` table.
        manager
            .alter_table(
                Table::alter().table(L1Message::Table).drop_column(L1Message::Skipped).to_owned(),
            )
            .await?;

        // Drop index `skipped` for the `l1_message` table.
        manager
            .drop_index(
                Index::drop().name("idx_l1_message_skipped").table(L1Message::Table).to_owned(),
            )
            .await?;

        Ok(())
    }
}
