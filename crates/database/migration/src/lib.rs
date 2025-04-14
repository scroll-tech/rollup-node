pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_batch_commit_table;
mod m20250304_125946_add_l1_msg_table;
mod m20250408_132123_add_block_data_table;
mod m20250408_150338_seed_block_data_table;
mod m20250411_072004_add_batch_to_block;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_batch_commit_table::Migration),
            Box::new(m20250304_125946_add_l1_msg_table::Migration),
            Box::new(m20250408_132123_add_block_data_table::Migration),
            Box::new(m20250408_150338_seed_block_data_table::Migration),
            Box::new(m20250411_072004_add_batch_to_block::Migration),
        ]
    }
}
