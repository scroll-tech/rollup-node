use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared("ANALYZE;").await?;
        db.execute_unprepared("DELETE FROM sqlite_stat1;").await?;
        db.execute_unprepared(
            r#"
                INSERT INTO sqlite_stat1(tbl, idx, stat) VALUES
                    ('block_signature', 'idx_block_signature_block_hash', '96134 1'),
                    ('block_signature', 'sqlite_autoindex_block_signature_1', '96134 1'),
                    ('metadata', 'sqlite_autoindex_metadata_1', '4 1'),
                    ('l2_block', 'idx_batch_index', '14240240 120'),
                    ('l2_block', 'idx_batch_hash', '14240240 120'),
                    ('l2_block', 'idx_block_hash', '14240240 1'),
                    ('l2_block', 'idx_l2_block_block_number', '14240240 1'),
                    ('block_data', 'sqlite_autoindex_block_data_1', '8484488 1'),
                    ('l1_message', 'idx_l1_message_queue_index', '1079892 1'),
                    ('l1_message', 'idx_l1_message_l2_block', '1079892 8'),
                    ('l1_message', 'idx_l1_block_number', '1079892 3'),
                    ('l1_message', 'idx_l1_message_hash', '1079892 1'),
                    ('l1_message', 'idx_queue_hash', '1079892 61'),
                    ('batch_commit', 'idx_finalized_block_number', '118854 2'),
                    ('batch_commit', 'idx_batch_commit_block_number', '118854 2'),
                    ('batch_commit', 'idx_batch_commit_hash', '118854 1'),
                    ('batch_commit', 'sqlite_autoindex_batch_commit_1', '118854 1'),
                    ('seaql_migrations', 'sqlite_autoindex_seaql_migrations_1', '17 1');
            "#,
        )
        .await?;
        db.execute_unprepared("ANALYZE sqlite_schema;").await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
