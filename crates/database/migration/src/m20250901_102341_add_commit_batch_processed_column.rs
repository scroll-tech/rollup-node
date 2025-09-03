use super::m20220101_000001_create_batch_commit_table::BatchCommit;
use sea_orm::Statement;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add the processed column to the batch_commit table.
        manager
            .alter_table(
                Table::alter()
                    .table(BatchCommit::Table)
                    .add_column(
                        ColumnDef::new(BatchCommit::Processed).boolean().not_null().default(false),
                    )
                    .to_owned(),
            )
            .await?;

        // Backfill the processed column using data sourced from the l2_block table.
        manager
            .get_connection()
            .execute(Statement::from_sql_and_values(
                manager.get_database_backend(),
                r#"
                UPDATE batch_commit
                SET processed = 1
                WHERE hash IN (SELECT batch_hash FROM l2_block);
                "#,
                vec![],
            ))
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // drop the processed column on the batch_commit table.
        manager
            .alter_table(
                Table::alter()
                    .table(BatchCommit::Table)
                    .drop_column(BatchCommit::Processed)
                    .to_owned(),
            )
            .await
    }
}
