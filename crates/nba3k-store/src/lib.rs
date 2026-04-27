//! SQLite persistence layer for nba3k-claude.

mod store;

pub use store::*;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("migration: {0}")]
    Migration(#[from] refinery::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

mod embedded {
    refinery::embed_migrations!("migrations");
}

pub use embedded::migrations as MIGRATIONS;
