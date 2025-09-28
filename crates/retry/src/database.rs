use crate::{
    request::{DatabaseRequest, Scope},
    tx::{DatabaseTransactionProvider, TXMut, TX},
    DatabaseConnectionProvider, DatabaseError,
};

use sea_orm::{
    sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    DatabaseConnection, EntityTrait, SqlxSqliteConnector, TransactionTrait,
};
use std::{
    future::Future,
    pin::Pin,
    str::FromStr,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::Mutex;
use tower::{Layer, Service};

pub enum SingleOrMultiple<T> {
    Single(Option<T>),
    Multiple(Vec<T>),
}

impl<T> From<Option<T>> for SingleOrMultiple<T> {
    fn from(value: Option<T>) -> Self {
        SingleOrMultiple::Single(value)
    }
}

impl<T> From<Vec<T>> for SingleOrMultiple<T> {
    fn from(value: Vec<T>) -> Self {
        SingleOrMultiple::Multiple(value)
    }
}

impl<T: EntityTrait> Service<DatabaseRequest<T>> for Database {
    type Response = SingleOrMultiple<T::Model>;
    type Error = DatabaseError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: DatabaseRequest<T>) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            match req {
                DatabaseRequest::Select { entity, scope, .. } => {
                    let tx = this.tx().await?;
                    match scope {
                        Scope::All => Ok(entity.all(tx.get_connection()).await?.into()),
                        Scope::One => Ok(entity.one(tx.get_connection()).await?.into()),
                    }
                }
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct Database {
    /// The underlying database connection.
    connection: DatabaseConnection,
    /// A mutex to ensure that only one mutable transaction is active at a time.
    write_lock: Arc<Mutex<()>>,
}

impl Database {
    /// Creates a new [`Database`] instance associated with the provided database URL.
    pub async fn new(database_url: &str) -> Result<Self, DatabaseError> {
        Self::new_sqlite_with_pool_options(database_url, 10, 1, 5, 5).await
    }

    /// Creates a new [`Database`] instance with SQLite-specific optimizations and custom pool
    /// settings.
    pub async fn new_sqlite_with_pool_options(
        database_url: &str,
        max_connections: u32,
        min_connections: u32,
        acquire_timeout_secs: u64,
        busy_timeout_secs: u64,
    ) -> Result<Self, DatabaseError> {
        let options = SqliteConnectOptions::from_str(database_url)
            .unwrap()
            .create_if_missing(true)
            .journal_mode(sea_orm::sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(busy_timeout_secs))
            .foreign_keys(true)
            .synchronous(sea_orm::sqlx::sqlite::SqliteSynchronous::Normal);

        let sqlx_pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .min_connections(min_connections)
            .acquire_timeout(Duration::from_secs(acquire_timeout_secs))
            .connect_with(options)
            .await
            .unwrap();

        Ok(Self {
            connection: SqlxSqliteConnector::from_sqlx_sqlite_pool(sqlx_pool),
            write_lock: Arc::new(Mutex::new(())),
        })
    }
}

#[async_trait::async_trait]
impl DatabaseTransactionProvider for Database {
    /// Creates a new [`TX`] which can be used for read-only operations.
    async fn tx(&self) -> Result<TX, DatabaseError> {
        Ok(TX::new(self.connection.clone().begin().await?))
    }

    /// Creates a new [`TXMut`] which can be used for atomic read and write operations.
    async fn tx_mut(&self) -> Result<TXMut, DatabaseError> {
        let now = std::time::Instant::now();
        let guard = self.write_lock.clone().lock_owned().await;
        let tx_mut = TXMut::new(self.connection.clone().begin().await?, guard);
        let duration = now.elapsed().as_millis() as f64;
        Ok(tx_mut)
    }
}

#[async_trait::async_trait]
impl DatabaseTransactionProvider for Arc<Database> {
    /// Creates a new [`TX`] which can be used for read-only operations.
    async fn tx(&self) -> Result<TX, DatabaseError> {
        self.as_ref().tx().await
    }

    /// Creates a new [`TXMut`] which can be used for atomic read and write operations.
    async fn tx_mut(&self) -> Result<TXMut, DatabaseError> {
        self.as_ref().tx_mut().await
    }
}

impl DatabaseConnectionProvider for Database {
    type Connection = DatabaseConnection;

    fn get_connection(&self) -> &Self::Connection {
        &self.connection
    }
}
