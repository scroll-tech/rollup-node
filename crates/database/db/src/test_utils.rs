//! Test utilities for the database crate.

use crate::DatabaseConnectionProvider;

use super::Database;
use scroll_migration::{Migrator, MigratorTrait, ScrollDevMigrationInfo};
use tempfile::NamedTempFile;

/// Instantiates a new in-memory database and runs the migrations
/// to set up the schema.
pub async fn setup_test_db() -> Database {
    let tmp = NamedTempFile::new().unwrap();
    let db = Database::new(tmp.path().to_str().unwrap()).await.unwrap();
    Migrator::<ScrollDevMigrationInfo>::up(db.get_connection(), None).await.unwrap();
    db
}
