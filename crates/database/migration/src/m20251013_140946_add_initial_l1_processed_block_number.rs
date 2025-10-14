use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Insert both keys if they don't already exist
        db.execute_unprepared(
            r#"
            INSERT INTO metadata (key, value)
            VALUES 
                ('l1_processed_block', '0')
            ON CONFLICT(key) DO NOTHING;
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            DELETE FROM metadata 
            WHERE key = 'l1_processed_block';
            "#,
        )
        .await?;

        Ok(())
    }
}
