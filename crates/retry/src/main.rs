use random::database::Database;
use random::retry::RetryBackoffLayer;
use tower::{Layer, ServiceBuilder};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let db = Database::new("test.db").await?;
    let retry = RetryBackoffLayer::default().layer(db);

    Ok(())
}
