//! Test utilities for the database crate.

use crate::DatabaseConnectionProvider;

use super::Database;
use scroll_migration::{Migrator, MigratorTrait, ScrollDevMigrationInfo};

/// Instantiates a new in-memory database and runs the migrations
/// to set up the schema.
pub async fn setup_test_db() -> Database {
    let dir = tempfile::Builder::new()
        .prefix("scroll-test-")
        .rand_bytes(8)
        .tempdir()
        .expect("failed to create temp dir");
    let db = Database::test(dir).await.unwrap();
    Migrator::<ScrollDevMigrationInfo>::up(db.inner().get_connection(), None).await.unwrap();
    db
}
