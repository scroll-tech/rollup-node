//! A library responsible for interacting with the database.

mod connection;
pub use connection::DatabaseConnectionProvider;

mod db;
pub use db::Database;

mod error;
pub use error::DatabaseError;

mod models;
pub use models::*;

mod operations;
pub use operations::{DatabaseOperations, UnwindResult};

mod transaction;
pub use transaction::DatabaseTransaction;

#[cfg(feature = "test-utils")]
pub mod test_utils;
