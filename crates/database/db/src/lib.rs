//! A library responsible for interacting with the database.

mod connection;
pub use connection::{DatabaseConnectionProvider, ReadConnectionProvider, WriteConnectionProvider};

mod db;
pub use db::Database;

mod error;
pub use error::DatabaseError;

mod metrics;

mod models;
pub use models::*;

mod operations;
pub use operations::{
    DatabaseReadOperations, DatabaseWriteOperations, L1MessageKey, NotIncludedStart, UnwindResult,
};

mod transaction;
pub use transaction::{DatabaseTransactionProvider, TXMut, TX};

#[cfg(feature = "test-utils")]
pub mod test_utils;
