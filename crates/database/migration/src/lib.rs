pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_batch_commit_table;
mod m20250304_125946_add_l1_msg_table;
mod m20250408_132123_add_header_metadata;
mod m20250408_150338_load_header_metadata;
mod m20250411_072004_add_l2_block;
mod m20250616_223947_add_metadata;
mod m20250825_093350_remove_unsafe_l2_blocks;
mod m20250829_042803_add_table_indexes;
mod m20250901_102341_add_commit_batch_processed_column;
mod migration_info;
pub use migration_info::{
    MigrationInfo, ScrollDevMigrationInfo, ScrollMainnetMigrationInfo, ScrollSepoliaMigrationInfo,
};

pub struct Migrator<MI>(pub std::marker::PhantomData<MI>);

#[async_trait::async_trait]
impl<MI: MigrationInfo + Send + Sync + 'static> MigratorTrait for Migrator<MI> {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_batch_commit_table::Migration),
            Box::new(m20250304_125946_add_l1_msg_table::Migration),
            Box::new(m20250408_132123_add_header_metadata::Migration),
            Box::new(m20250408_150338_load_header_metadata::Migration::<MI>(Default::default())),
            Box::new(m20250411_072004_add_l2_block::Migration::<MI>(Default::default())),
            Box::new(m20250616_223947_add_metadata::Migration),
            Box::new(m20250825_093350_remove_unsafe_l2_blocks::Migration),
            Box::new(m20250829_042803_add_table_indexes::Migration),
            Box::new(m20250901_102341_add_commit_batch_processed_column::Migration),
        ]
    }
}

pub mod traits {
    use crate::{
        migration_info::{ScrollDevMigrationInfo, ScrollMainnetTestMigrationInfo},
        ScrollMainnetMigrationInfo, ScrollSepoliaMigrationInfo,
    };
    use reth_chainspec::NamedChain;
    use sea_orm::{prelude::async_trait::async_trait, DatabaseConnection, DbErr};
    use sea_orm_migration::MigratorTrait;

    /// An instance of the trait can perform the migration for Scroll.
    #[async_trait]
    pub trait ScrollMigrator {
        /// Migrates the tables.
        async fn migrate(&self, conn: &DatabaseConnection, test: bool) -> Result<(), DbErr>;
    }

    #[async_trait]
    impl ScrollMigrator for NamedChain {
        async fn migrate(&self, conn: &DatabaseConnection, test: bool) -> Result<(), DbErr> {
            match (self, test) {
                (NamedChain::Scroll, false) => {
                    Ok(super::Migrator::<ScrollMainnetMigrationInfo>::up(conn, None))
                }
                (NamedChain::Scroll, true) => {
                    Ok(super::Migrator::<ScrollMainnetTestMigrationInfo>::up(conn, None))
                }
                (NamedChain::ScrollSepolia, _) => {
                    Ok(super::Migrator::<ScrollSepoliaMigrationInfo>::up(conn, None))
                }
                (NamedChain::Dev, _) => {
                    Ok(super::Migrator::<ScrollDevMigrationInfo>::up(conn, None))
                }
                _ => Err(DbErr::Custom("expected Scroll Mainnet, Sepolia or Dev".into())),
            }?
            .await
        }
    }
}
