//! A library responsible for interacting with the database.

mod models;
pub use models::*;

mod db;
pub use db::Database;

#[cfg(test)]
pub mod test_utils;
