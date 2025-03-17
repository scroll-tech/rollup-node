//! Test utilities for the database crate.

use super::Database;
use migration::{Migrator, MigratorTrait};
use sea_orm::DatabaseConnection;

/// Instantiates a new in-memory database and runs the migrations
/// to set up the schema.
pub async fn setup_test_db() -> Database<DatabaseConnection> {
    let database_url = "sqlite::memory:";
    let connection = sea_orm::Database::connect(database_url).await.unwrap();
    Migrator::up(&connection, None).await.unwrap();

    Database { connection }
}
