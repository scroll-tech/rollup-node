//! A library responsible for interacting with the database.

mod models;
pub use models::*;

mod db;
pub use db::Database;

#[cfg(feature = "test-utils")]
pub mod test_utils;

pub use sea_orm::DbErr;
