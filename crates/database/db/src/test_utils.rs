//! Test utilities for the database crate.

use super::Database;
use migration::{GetBlockDataUrl, Migrator, MigratorTrait};

pub struct ScrollMainnetGBD;

impl GetBlockDataUrl for ScrollMainnetGBD {
    fn get_block_data_url() -> String {
        "https://blockdata.scroll.io/mainnet/".to_string()
    }
}

pub struct ScrollTestnetGBD;

impl GetBlockDataUrl for ScrollTestnetGBD {
    fn get_block_data_url() -> String {
        "https://blockdata.scroll.io/testnet/".to_string()
    }
}

/// Instantiates a new in-memory database and runs the migrations
/// to set up the schema.
pub async fn setup_test_db() -> Database {
    let database_url = "sqlite::memory:";
    let connection = sea_orm::Database::connect(database_url).await.unwrap();
    Migrator::<ScrollMainnetGBD>::up(&connection, None).await.unwrap();
    connection.into()
}
