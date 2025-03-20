use sea_orm_migration::prelude::*;

#[async_std::main]
async fn main() {
    tracing::info!(target: "scroll::migration", "Running database migrations.");
    cli::run_cli(migration::Migrator).await;
    tracing::info!(target: "scroll::migration", "Database migrations complete.")
}
