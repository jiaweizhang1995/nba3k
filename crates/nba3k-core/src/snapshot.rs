//! `LeagueSnapshot` — read-only view of league state. Lives in core so every
//! downstream crate (trade, sim, models, season) can consume it without
//! creating dependency cycles.
//!
//! The trade engine still re-exports this type at
//! `nba3k_trade::snapshot::*` for backward compatibility — the relocation
//! preserves all existing public paths.

use crate::{
    DraftPick, DraftPickId, LeagueYear, Player, PlayerId, SeasonId, SeasonPhase, Team, TeamId,
};
use chrono::NaiveDate;
use std::collections::HashMap;

/// Compact per-team standings view used by trade-engine and sim consumers.
#[derive(Debug, Clone, Copy)]
pub struct TeamRecordSummary {
    pub wins: u16,
    pub losses: u16,
    /// 1-indexed within conference. 0 = unknown / not yet computed.
    pub conf_rank: u8,
    /// Cumulative point differential, signed.
    pub point_diff: i32,
}

impl TeamRecordSummary {
    pub fn games_played(&self) -> u16 {
        self.wins + self.losses
    }

    pub fn win_pct(&self) -> f32 {
        let gp = self.games_played();
        if gp == 0 {
            0.5
        } else {
            self.wins as f32 / gp as f32
        }
    }
}

impl Default for TeamRecordSummary {
    fn default() -> Self {
        Self {
            wins: 0,
            losses: 0,
            conf_rank: 0,
            point_diff: 0,
        }
    }
}

/// Read-only view of league state. Everything is borrowed — caller owns the
/// underlying buffers and rebuilds the snapshot per command.
#[derive(Debug, Clone, Copy)]
pub struct LeagueSnapshot<'a> {
    pub current_season: SeasonId,
    pub current_phase: SeasonPhase,
    pub current_date: NaiveDate,
    pub league_year: LeagueYear,
    pub teams: &'a [Team],
    pub players_by_id: &'a HashMap<PlayerId, Player>,
    pub picks_by_id: &'a HashMap<DraftPickId, DraftPick>,
    pub standings: &'a HashMap<TeamId, TeamRecordSummary>,
}

impl<'a> LeagueSnapshot<'a> {
    pub fn team(&self, id: TeamId) -> Option<&Team> {
        self.teams.iter().find(|t| t.id == id)
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players_by_id.get(&id)
    }

    pub fn pick(&self, id: DraftPickId) -> Option<&DraftPick> {
        self.picks_by_id.get(&id)
    }

    pub fn record(&self, id: TeamId) -> TeamRecordSummary {
        self.standings.get(&id).copied().unwrap_or_default()
    }

    /// All players currently on `team`'s roster (active = team_id matches).
    /// Sorted by `(overall desc, id asc)` so callers get a stable order
    /// across HashMap iteration shuffles.
    pub fn roster(&self, team: TeamId) -> Vec<&Player> {
        let mut out: Vec<&Player> = self
            .players_by_id
            .values()
            .filter(|p| p.team == Some(team))
            .collect();
        out.sort_by(|a, b| b.overall.cmp(&a.overall).then_with(|| a.id.0.cmp(&b.id.0)));
        out
    }
}
