//! A library responsible for interacting with the database.

mod error;
pub use error::DatabaseError;

mod db;
pub use db::Database;

mod models;
pub use models::*;

#[cfg(feature = "test-utils")]
pub mod test_utils;
