use super::{
    m20220101_000001_create_batch_commit_table::BatchCommit,
    m20250304_125946_add_l1_msg_table::L1Message, m20250411_072004_add_l2_block::L2Block,
};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create indexes for the `batch_commit` table.
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_hash")
                    .col(BatchCommit::Hash)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_commit_block_number")
                    .col(BatchCommit::BlockNumber)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_finalized_block_number")
                    .col(BatchCommit::FinalizedBlockNumber)
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        // Create indexes for the L1 message table.
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
            .await?;

        // Create indexes for L2 block table.
        manager
            .create_index(
                Index::create()
                    .name("idx_l2_block_block_number")
                    .col(L2Block::BlockNumber)
                    .table(L2Block::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_block_hash")
                    .col(L2Block::BlockHash)
                    .table(L2Block::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_hash")
                    .col(L2Block::BatchHash)
                    .table(L2Block::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_batch_index")
                    .col(L2Block::BatchIndex)
                    .table(L2Block::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop indexes for the `batch_commit` table.
        manager
            .drop_index(
                Index::drop().name("idx_batch_commit_hash").table(BatchCommit::Table).to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx_batch_commit_block_number")
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx_finalized_block_number")
                    .table(BatchCommit::Table)
                    .to_owned(),
            )
            .await?;

        // Drop indexes for the L1 message table.
        manager
            .drop_index(Index::drop().name("idx_queue_hash").table(L1Message::Table).to_owned())
            .await?;
        manager
            .drop_index(
                Index::drop().name("idx_l1_message_hash").table(L1Message::Table).to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop().name("idx_l1_block_number").table(L1Message::Table).to_owned(),
            )
            .await?;

        // Drop indexes for L2 block table.
        manager
            .drop_index(
                Index::drop().name("idx_l2_block_block_number").table(L2Block::Table).to_owned(),
            )
            .await?;
        manager
            .drop_index(Index::drop().name("idx_block_hash").table(L2Block::Table).to_owned())
            .await?;
        manager
            .drop_index(Index::drop().name("idx_batch_hash").table(L2Block::Table).to_owned())
            .await?;
        manager
            .drop_index(Index::drop().name("idx_batch_index").table(L2Block::Table).to_owned())
            .await?;

        Ok(())
    }
}
