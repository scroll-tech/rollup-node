//! A library responsible for interacting with the database.

mod models;
pub use models::*;

mod db;
pub use db::Database;

mod iterator;
pub use iterator::DatabaseEntryStream;

pub use sea_orm::DbErr;

/// A module for reusable database test utilities.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
