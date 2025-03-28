pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_batch_input_table;
mod m20250304_125946_add_l1_msg_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_batch_input_table::Migration),
            Box::new(m20250304_125946_add_l1_msg_table::Migration),
        ]
    }
}
