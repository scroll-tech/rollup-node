use super::{m20220101_000001_create_batch_commit_table::BatchCommit, MigrationInfo};

use sea_orm::Statement;
use sea_orm_migration::{prelude::*, schema::*};

pub struct Migration<MI>(pub std::marker::PhantomData<MI>);

impl<MI> MigrationName for Migration<MI> {
    fn name(&self) -> &str {
        sea_orm_migration::util::get_file_stem(file!())
    }
}

#[async_trait::async_trait]
impl<MI: MigrationInfo + Send + Sync> MigrationTrait for Migration<MI> {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(L2Block::Table)
                    .if_not_exists()
                    .col(pk_auto(L2Block::BlockNumber))
                    .col(binary_len(L2Block::BlockHash, 32))
                    .col(big_unsigned(L2Block::BatchIndex))
                    .col(binary_len(L2Block::BatchHash, 32))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_batch_index")
                            .from(L2Block::Table, L2Block::BatchIndex)
                            .to(BatchCommit::Table, BatchCommit::Index)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_batch_hash")
                            .from(L2Block::Table, L2Block::BatchHash)
                            .to(BatchCommit::Table, BatchCommit::Hash)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Insert the genesis block.
        let genesis_hash = MI::genesis_hash();

        manager
            .get_connection()
            .execute(Statement::from_sql_and_values(
                manager.get_database_backend(),
                r#"
        INSERT INTO l2_block (block_number, block_hash, batch_index, batch_hash)
        VALUES (?, ?, ?, ?)
        "#,
                vec![0u64.into(), genesis_hash.to_vec().into(), 0u64.into(), vec![0u8; 32].into()],
            ))
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(L2Block::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum L2Block {
    Table,
    BatchIndex,
    BatchHash,
    BlockNumber,
    BlockHash,
}
