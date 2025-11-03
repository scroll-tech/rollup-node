use super::m20250904_175949_block_signature::BlockSignature;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create indexes for the `block_signature` table.
        manager
            .create_index(
                Index::create()
                    .name("idx_block_signature_block_hash")
                    .col(BlockSignature::BlockHash)
                    .table(BlockSignature::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop indexes for the `block_signature` table.
        manager
            .drop_index(
                Index::drop()
                    .name("idx_block_signature_block_hash")
                    .table(BlockSignature::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
