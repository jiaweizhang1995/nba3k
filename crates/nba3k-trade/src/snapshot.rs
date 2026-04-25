//! Backward-compat re-export. The canonical `LeagueSnapshot` and
//! `TeamRecordSummary` types now live in `nba3k_core` so the M4 realism
//! engine can consume them without creating a dependency cycle. All
//! existing call sites that import from `nba3k_trade::snapshot::*`
//! continue to work unchanged.

pub use nba3k_core::{LeagueSnapshot, TeamRecordSummary};
