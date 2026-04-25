//! Trade engine. Headline feature.
//!
//! Layout:
//! - `snapshot`     — `LeagueSnapshot` (read-only view consumed by every other module).
//! - `evaluate`     — Worker A: per-evaluator value + Verdict.
//! - `valuation`    — Worker A: player + pick + cash valuation primitives.
//! - `personality`  — Worker B: load 30 hand-tuned GMs from TOML.
//! - `context`      — Worker B: TeamMode classifier + trait modulation.
//! - `cba`          — Worker C: Standard-mode CBA validator.
//! - `negotiate`    — Worker D: counter-offer state machine.
//!
//! Public API stability matters: CLI consumes the top-level functions in each
//! module via the re-exports below.

pub mod cba;
pub mod context;
pub mod evaluate;
pub mod negotiate;
pub mod personality;
pub mod snapshot;
pub mod valuation;

pub use snapshot::{LeagueSnapshot, TeamRecordSummary};

use serde::{Deserialize, Serialize};

/// Coarse team posture used to modulate trait weights at evaluation time.
/// Never persisted — recomputed per evaluation from the live snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TeamMode {
    FullRebuild,
    SoftRebuild,
    Retool,
    Contend,
    Tank,
}

#[derive(Debug, thiserror::Error)]
pub enum TradeError {
    #[error("invalid offer: {0}")]
    InvalidOffer(String),
    #[error("missing data: {0}")]
    MissingData(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
}

pub type TradeResult<T> = Result<T, TradeError>;
