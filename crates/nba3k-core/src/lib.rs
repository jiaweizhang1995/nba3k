//! Core domain types for nba3k-claude. Zero I/O. Pure data.

mod ids;
mod player;
mod team;
mod contract;
mod draft;
mod trade;
mod season;
mod sim_io;
mod gm;
mod coach;
mod money;
mod league_year;
mod snapshot;
pub mod rotation;

pub use ids::*;
pub use player::*;
pub use team::*;
pub use contract::*;
pub use draft::*;
pub use trade::*;
pub use season::*;
pub use sim_io::*;
pub use gm::*;
pub use coach::*;
pub use money::*;
pub use league_year::*;
pub use snapshot::*;
pub use rotation::Starters;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type CoreResult<T> = Result<T, CoreError>;
