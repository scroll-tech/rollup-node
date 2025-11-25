use sea_orm::ConnectionTrait;

use crate::DatabaseConnectionProvider;

use super::Database;
use std::sync::Arc;

/// The interval in seconds between optimization runs in seconds.
const PERIODIC_MAINTENANCE_INTERVAL_SECS: u64 = 600;

/// Provides maintenance operations for the database.
#[derive(Debug)]
pub struct DatabaseMaintenance {
    db: Arc<Database>,
}

impl DatabaseMaintenance {
    /// Creates a new `DatabaseMaintenance` instance.
    pub const fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Runs the maintenance tasks in a loop.
    pub async fn run(self) {
        self.startup_maintenance().await;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(PERIODIC_MAINTENANCE_INTERVAL_SECS))
                .await;
            self.periodic_maintenance().await;
        }
    }

    /// Runs maintenance tasks at startup.
    ///
    /// This includes running `ANALYZE` and `PRAGMA optimize`.
    async fn startup_maintenance(&self) {
        let db = self.db.inner();
        let conn = db.get_connection();

        tracing::info!(target: "scroll::db::maintenance", "running startup ANALYZE...");
        if let Err(err) = conn.execute_unprepared("ANALYZE;").await {
            tracing::warn!(target: "scroll::db::maintenance", "ANALYZE failed: {:?}", err);
        }

        tracing::info!(target: "scroll::db::maintenance", "running PRAGMA optimize at startup...");
        if let Err(err) = conn.execute_unprepared("PRAGMA optimize;").await {
            tracing::warn!(target: "scroll::db::maintenance", "PRAGMA optimize failed: {:?}", err);
        }

        tracing::info!(target: "scroll::db::maintenance", "startup maintenance complete.");
    }

    /// Runs periodic maintenance tasks.
    ///
    /// This includes running `PRAGMA optimize`.
    async fn periodic_maintenance(&self) {
        let db = self.db.inner();
        let conn = db.get_connection();

        tracing::info!(target: "scroll::db::maintenance", "running periodic PRAGMA optimize...");
        if let Err(err) = conn.execute_unprepared("PRAGMA optimize;").await {
            tracing::warn!(target: "scroll::db::maintenance", "PRAGMA optimize failed: {:?}", err);
        } else {
            tracing::info!(target: "scroll::db::maintenance", "periodic PRAGMA optimize complete.");
        }
    }
}
