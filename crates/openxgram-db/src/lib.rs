//! OpenXgram DB — SQLite + sqlite-vec, 마이그레이션, 트랜잭션

mod connection;
mod error;
mod migrate;
mod pragma;

pub mod schema;

pub use connection::{Db, DbConfig, JournalMode};
pub use error::DbError;
pub use migrate::{Migration, MigrationRecord, MigrationRunner};
