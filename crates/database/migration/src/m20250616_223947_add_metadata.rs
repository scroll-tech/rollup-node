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
            .await?;

        // Insert both keys if they don't already exist
        manager
            .get_connection()
            .execute_unprepared(
                r#"
            INSERT INTO metadata (key, value)
            VALUES 
                ('l1_finalized_block', '0'),
                ('l1_latest_block', '0'),
                ('l2_head_block', '0'),
                ('l1_processed_block', '0')
            ON CONFLICT(key) DO NOTHING;
            "#,
            )
            .await?;

        Ok(())
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
