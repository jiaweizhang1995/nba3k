//! Core domain types for nba3k-claude. Zero I/O. Pure data.

mod coach;
mod contract;
mod draft;
mod gm;
pub mod i18n;
mod i18n_en;
mod i18n_zh;
mod ids;
mod league_year;
mod money;
mod player;
pub mod rotation;
mod season;
mod sim_io;
mod snapshot;
mod team;
mod trade;

pub use coach::*;
pub use contract::*;
pub use draft::*;
pub use gm::*;
pub use i18n::{t, Lang, T};
pub use ids::*;
pub use league_year::*;
pub use money::*;
pub use player::*;
pub use rotation::Starters;
pub use season::*;
pub use sim_io::*;
pub use snapshot::*;
pub use team::*;
pub use trade::*;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type CoreResult<T> = Result<T, CoreError>;
