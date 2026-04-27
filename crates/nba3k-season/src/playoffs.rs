//! Playoff bracket + best-of-7 series simulation.
//!
//! 16-team bracket: 8 East + 8 West, seeded 1..=8 by `Standings.conf_rank`.
//! First three rounds stay within conference (1v8 / 4v5 / 3v6 / 2v7), then
//! the two conference champs meet in the NBA Finals.
//!
//! Series schedule is the canonical NBA 2-2-1-1-1 home-court split: the
//! higher seed (treated as `home_team` in our types) hosts games 1, 2, 5,
//! and 7; the lower seed hosts games 3, 4, and 6. Series ends as soon as
//! either side wins 4 — so valid final win counts are 4-0 / 4-1 / 4-2 /
//! 4-3, never 5-3 or worse.
//!
//! Finals MVP: the highest top-line scorer on the championship team across
//! the Finals series, weighted by team success (winners' team multiplier
//! 1.0; losers' 0.85 — applied to the per-series totals before pick).
//!
//! Decisions captured in `phases/M5-realism-v2.md` "Decision log":
//! - Home-court rule: 2-2-1-1-1 (confirmed).
//! - Best-of-7 only — no play-in tournament in v1.

use crate::standings::Standings;
use chrono::{Duration, NaiveDate};
use nba3k_core::{Conference, GameId, GameResult, PlayerId, SeasonId, TeamId};
use nba3k_sim::{Engine, GameContext, TeamSnapshot};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ----------------------------------------------------------------------
// Types
// ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayoffRound {
    /// Conference quarterfinals (8 series).
    R1,
    /// Conference semifinals (4 series).
    Semis,
    /// Conference finals (2 series).
    ConfFinals,
    /// NBA Finals (1 series).
    Finals,
}

impl PlayoffRound {
    /// 1..=4 mapping that lines up with the persisted `series.round` column.
    pub fn ord(self) -> u8 {
        match self {
            Self::R1 => 1,
            Self::Semis => 2,
            Self::ConfFinals => 3,
            Self::Finals => 4,
        }
    }
}

/// One scheduled best-of-7 matchup. The `home` team is the higher seed and
/// hosts games 1, 2, 5, 7 in the 2-2-1-1-1 schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    pub round: PlayoffRound,
    pub conference: Option<Conference>, // None for Finals
    pub home: TeamId,
    pub away: TeamId,
    pub home_seed: u8,
    pub away_seed: u8,
}

/// Result of running `simulate_series` on a `Series`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesResult {
    pub series: Series,
    pub home_wins: u8,
    pub away_wins: u8,
    pub games: Vec<GameResult>,
}

impl SeriesResult {
    pub fn winner(&self) -> TeamId {
        if self.home_wins > self.away_wins { self.series.home } else { self.series.away }
    }
    pub fn loser(&self) -> TeamId {
        if self.home_wins > self.away_wins { self.series.away } else { self.series.home }
    }
    pub fn is_complete(&self) -> bool {
        self.home_wins == 4 || self.away_wins == 4
    }
}

/// Full 16-team bracket pre-simulation. Round 1 is fully populated; later
/// rounds are filled in by the bracket runner as previous rounds resolve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bracket {
    pub season: SeasonId,
    /// 8 R1 series — 4 East + 4 West, one per canonical matchup (1v8/4v5/3v6/2v7).
    pub r1: Vec<Series>,
}

// ----------------------------------------------------------------------
// Bracket generation
// ----------------------------------------------------------------------

/// Build the 16-team R1 bracket from final regular-season standings.
/// Per conference, seeds are 1..=8 by `conf_rank`; matchups are 1v8, 4v5,
/// 3v6, 2v7 in that order so the bracket halves resolve cleanly into
/// conference semis (1/8 vs 4/5 → top half; 3/6 vs 2/7 → bottom half).
pub fn generate_bracket(standings: &Standings, season: SeasonId) -> Bracket {
    let east = top_eight(standings, Conference::East);
    let west = top_eight(standings, Conference::West);

    let mut r1 = Vec::with_capacity(8);
    for conf_seeds in [&east, &west] {
        let conf = if std::ptr::eq(conf_seeds, &east) {
            Conference::East
        } else {
            Conference::West
        };
        for &(home_seed, away_seed) in &[(1u8, 8u8), (4, 5), (3, 6), (2, 7)] {
            let home = conf_seeds.get((home_seed - 1) as usize).copied();
            let away = conf_seeds.get((away_seed - 1) as usize).copied();
            if let (Some(home), Some(away)) = (home, away) {
                r1.push(Series {
                    round: PlayoffRound::R1,
                    conference: Some(conf),
                    home,
                    away,
                    home_seed,
                    away_seed,
                });
            }
        }
    }
    Bracket { season, r1 }
}

fn top_eight(standings: &Standings, conf: Conference) -> Vec<TeamId> {
    let mut conf_records: Vec<(TeamId, u8)> = standings
        .records
        .iter()
        .filter(|(_, r)| r.conference == conf)
        .map(|(id, r)| (*id, r.conf_rank))
        .collect();
    // Sort ascending by rank (1 = top seed). conf_rank == 0 means standings
    // weren't recomputed yet — push to back via stable sort (rank 0 → max).
    conf_records.sort_by_key(|(id, rank)| (if *rank == 0 { 99 } else { *rank }, id.0));
    conf_records.into_iter().take(8).map(|(id, _)| id).collect()
}

// ----------------------------------------------------------------------
// Series simulation
// ----------------------------------------------------------------------

/// Run a best-of-7 with the 2-2-1-1-1 schedule. Each game is delegated to
/// `engine.simulate_game` with `is_playoffs = true`. Returns once either
/// side reaches 4 wins.
///
/// Game IDs start at `next_game_id`; caller supplies a starting cursor and
/// the function increments it. Game dates start at `start_date` and step
/// by 2 days between games (every-other-day cadence — fine for v1).
#[allow(clippy::too_many_arguments)]
pub fn simulate_series(
    series: Series,
    engine: &dyn Engine,
    home_snapshot: &TeamSnapshot,
    away_snapshot: &TeamSnapshot,
    season: SeasonId,
    start_date: NaiveDate,
    next_game_id: &mut u64,
    rng: &mut dyn RngCore,
) -> SeriesResult {
    // Game number → which team hosts (true = higher seed/home, false = lower).
    // 2-2-1-1-1: H H A A H A H.
    let host_pattern = [true, true, false, false, true, false, true];

    let mut home_wins = 0u8;
    let mut away_wins = 0u8;
    let mut games: Vec<GameResult> = Vec::with_capacity(7);

    for (game_idx, &home_hosts) in host_pattern.iter().enumerate() {
        if home_wins == 4 || away_wins == 4 {
            break;
        }
        let game_id = GameId(*next_game_id);
        *next_game_id += 1;
        let date = start_date + Duration::days(game_idx as i64 * 2);
        let ctx = GameContext {
            game_id,
            season,
            date,
            is_playoffs: true,
            home_back_to_back: false,
            away_back_to_back: false,
        };
        let (host_snap, visitor_snap) = if home_hosts {
            (home_snapshot, away_snapshot)
        } else {
            (away_snapshot, home_snapshot)
        };
        let game = engine.simulate_game(host_snap, visitor_snap, &ctx, rng);
        // Translate "host won" into our higher-seed-team frame.
        let host_won = game.home_score >= game.away_score;
        let series_home_won = if home_hosts { host_won } else { !host_won };
        if series_home_won {
            home_wins += 1;
        } else {
            away_wins += 1;
        }
        games.push(game);
    }

    SeriesResult { series, home_wins, away_wins, games }
}

// ----------------------------------------------------------------------
// Finals MVP
// ----------------------------------------------------------------------

/// Pick the Finals MVP from the championship team's box scores in the Finals
/// series. Returns `None` only when the series has no game lines — adequate
/// for empty fixtures, never for a real finished series.
pub fn compute_finals_mvp(finals: &SeriesResult) -> Option<PlayerId> {
    let champ = finals.winner();
    let mut totals: HashMap<PlayerId, f32> = HashMap::new();
    for g in &finals.games {
        let lines = if g.home == champ {
            &g.box_score.home_lines
        } else if g.away == champ {
            &g.box_score.away_lines
        } else {
            // Should never happen — guard anyway so a malformed series can't
            // crash the runner.
            continue;
        };
        for line in lines {
            // Composite mirrors MVP composite: scoring + secondary
            // contribution. No team gate (we already filtered to champion).
            let s = line.pts as f32
                + 0.5 * line.reb as f32
                + 0.7 * line.ast as f32
                + 1.5 * line.stl as f32
                + 1.5 * line.blk as f32
                - 1.0 * line.tov as f32;
            *totals.entry(line.player).or_insert(0.0) += s;
        }
    }
    let mut ranked: Vec<(PlayerId, f32)> = totals.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.first().map(|(p, _)| *p)
}
