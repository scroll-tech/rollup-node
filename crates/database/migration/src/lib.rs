pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_batch_commit_table;
mod m20250304_125946_add_l1_msg_table;
mod m20250408_132123_add_header_metadata;
mod m20250408_150338_load_header_metadata;
mod m20250411_072004_add_l2_block;
mod migration_info;
pub use migration_info::{MigrationInfo, ScrollMainnetMigrationInfo, ScrollSepoliaMigrationInfo};

pub struct Migrator<MI>(pub std::marker::PhantomData<MI>);

#[async_trait::async_trait]
impl<MI: MigrationInfo + Send + Sync + 'static> MigratorTrait for Migrator<MI> {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_batch_commit_table::Migration),
            Box::new(m20250304_125946_add_l1_msg_table::Migration),
            Box::new(m20250408_132123_add_header_metadata::Migration),
            Box::new(m20250408_150338_load_header_metadata::Migration::<MI>(Default::default())),
            Box::new(m20250411_072004_add_l2_block::Migration),
        ]
    }
}
